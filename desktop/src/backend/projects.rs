//! Local project metadata. No launch, provider, shell, or trust-setting effects.
use super::*;
const FIELDS:[&str;5]=["title","detail","url","cwd","status"];
fn object_keys(value:&Value,allowed:&[&str])->Result<()>{
    let fields=value.as_object().ok_or(Error::new("Project fields must be an object."))?;
    if fields.keys().any(|key|!allowed.contains(&key.as_str())){return Err(Error::new("Unknown project field."));}Ok(())
}
fn project_id(value:&Value)->Result<&str>{
    value.as_str().filter(|id|!id.trim().is_empty()&&id.chars().count()<=160&&!id.chars().any(char::is_control)).ok_or(Error::new("Supply the exact local project ID."))
}
fn position(state:&Value,id:&str)->Result<usize>{
    arr(&state["notes"]).iter().position(|row|row["id"]==id&&row["source"]=="notes").ok_or(Error::new("Local project no longer exists."))
}
fn field_token(state:&Value,project:&Value,field:&str)->Result<String>{
    let value=if field=="status"{json!({"status":project["status"],"override":state["overrides"][text(project,"id")]["status"]})}else{project[field].clone()};
    Ok(context_kit::sha256(&serde_json::to_vec(&value)?))
}
fn view(state:&Value,index:usize)->Result<Value>{
    let row=&state["notes"][index];let id=text(row,"id");let mut project=json!({"id":id,"source":"notes","kind":"Project"});
    for (field,bound) in [("title",300),("detail",10000),("url",4096),("cwd",4096)]{
        let value=row[field].as_str().unwrap_or("");if value.chars().count()>bound{return Err(Error::new("Stored project metadata exceeds its editable limit."));}project[field]=json!(value);
    }
    let status=state["overrides"][id]["status"].as_str().filter(|status|!status.is_empty()).or(row["status"].as_str()).unwrap_or("todo");
    if status.chars().count()>32{return Err(Error::new("Stored project status exceeds its editable limit."));}project["status"]=json!(status);
    for field in ["updatedAt","archivedAt"]{project[field]=row[field].as_f64().filter(|time|time.is_finite()&&*time>=0.0).map_or(Value::Null,|time|json!(time));}
    let mut expected=json!({});for field in FIELDS{expected[field]=json!(field_token(state,row,field)?);}
    Ok(json!({"project":project,"expected":expected,"scope":"local"}))
}
fn normalize(fields:&Value,create:bool)->Result<Value>{
    object_keys(fields,&FIELDS)?;let mut result=json!({});
    for field in FIELDS {
        let Some(value)=fields.get(field) else{continue;};if !value.is_string(){return Err(Error::new(format!("Invalid project {field}.")));}
        let limit=match field{"title"=>300,"detail"=>10000,"status"=>32,_=>4096};let value=checked(fields,field,limit,field=="title")?;
        result[field]=json!(match field{"url"=>safe_url(&value)?,"cwd" if !value.is_empty()=>directory(&value)?,"status"=>{if !domain::STATUSES.contains(&value.as_str()){return Err(Error::new("Invalid project status."));}value},_=>value});
    }
    if create {
        if result.get("title").is_none(){return Err(Error::new("Supply a project title."));}
        for field in ["detail","url","cwd"]{if result.get(field).is_none(){result[field]=json!("");}}
        if result.get("status").is_none(){result["status"]=json!("todo");}
    }else if result.as_object().unwrap().is_empty(){return Err(Error::new("Supply at least one project change."));}
    Ok(result)
}
fn expected_keys(patch:&Value,expected:&Value)->Result<()>{
    let expected=expected.as_object().ok_or(Error::new("Supply captured tokens for exactly the changed project fields."))?;
    if expected.len()!=patch.as_object().unwrap().len()||patch.as_object().unwrap().keys().any(|key|!expected.contains_key(key))||expected.values().any(|value|!value.as_str().is_some_and(|hash|hash.len()==64&&hash.bytes().all(|byte|byte.is_ascii_hexdigit()))){return Err(Error::new("Supply captured tokens for exactly the changed project fields."));}Ok(())
}
fn create_normalized(state:&mut Value,fields:&Value)->Result<Value>{
    let mut row=fields.clone();row["id"]=json!(format!("note:{}",uuid()));row["source"]=json!("notes");row["kind"]=json!("Project");row["project"]=json!("Personal");row["reference"]=json!("");row["sourceStatus"]=json!("");row["archivedAt"]=Value::Null;row["updatedAt"]=json!(now());
    let rows=state["notes"].as_array_mut().ok_or(Error::new("Invalid local project storage."))?;rows.push(row);let index=rows.len()-1;let mut result=view(state,index)?;result["saved"]=json!(true);result["created"]=json!(true);result["changedFields"]=json!(FIELDS);Ok(result)
}
fn update_normalized(state:&mut Value,id:&str,patch:&Value,expected:&Value)->Result<Value>{
    expected_keys(patch,expected)?;let index=position(state,id)?;
    for field in patch.as_object().unwrap().keys(){if expected[field]!=field_token(state,&state["notes"][index],field)?{let mut error=Error::new("This project field changed. Read the project again and reapply only your intended changes.");error.status=409;return Err(error);}}
    let mut changed=vec![];
    for (field,value) in patch.as_object().unwrap(){let cleared_override=field=="status"&&state["overrides"][id].get("status").is_some();if state["notes"][index][field]!=*value||cleared_override{changed.push(field.clone());state["notes"][index][field]=value.clone();}if field=="status"{if let Some(overrides)=state["overrides"][id].as_object_mut(){overrides.remove("status");}}}
    if !changed.is_empty(){state["notes"][index]["updatedAt"]=json!(now());}
    let mut result=view(state,index)?;result["saved"]=json!(true);result["changedFields"]=json!(changed);Ok(result)
}
pub fn read(repo:&Repository,args:&Value)->Result<Value>{object_keys(args,&["id"])?;let id=project_id(&args["id"])?;let state=repo.state.lock().map_err(|_|Error::io())?;view(&state,position(&state,id)?)}
pub fn folder(repo:&Repository,args:&Value)->Result<Value>{folder_with(repo,args,&super::workflows::work_root())}
fn folder_with(repo:&Repository,args:&Value,root:&std::path::Path)->Result<Value>{
    object_keys(args,&["id"])?;let id=project_id(&args["id"])?;
    let (project,context)={let state=repo.state.lock().map_err(|_|Error::io())?;let project=state["notes"][position(&state,id)?].clone();let context=json!({"itemSettings":{id:state["itemSettings"][id]},"folderMappings":state["folderMappings"]});(project,context)};
    let cwd=super::workflows::automatic_folder(&context,&project,root,&json!([]));
    Ok(json!({"id":id,"requiresSelection":cwd.is_none(),"cwd":cwd}))
}
pub fn create(repo:&Repository,args:&Value)->Result<Value>{let fields=normalize(args,true)?;repo.transaction(|state|create_normalized(state,&fields))}
pub fn update(repo:&Repository,args:&Value)->Result<Value>{object_keys(args,&["id","patch","expected"])?;let id=project_id(&args["id"])?;let patch=normalize(&args["patch"],false)?;expected_keys(&patch,&args["expected"])?;repo.transaction_if_changed(|state|update_normalized(state,id,&patch,&args["expected"]))}
/// Compatibility transport only; existing legacy updates must carry the same captured tokens.
/// Creation from workflows uses this same validator and canonical record owner.
pub(super) fn save_in_state(state:&mut Value,args:&Value)->Result<Value>{
    object_keys(args,&["action","id","title","detail","url","cwd","status","patch","expected"])?;
    if args.get("id").is_some_and(|id|!id.is_string()){return Err(Error::new("Invalid project ID."));}
    let id=text(args,"id");let mut fields=json!({});for field in FIELDS{if let Some(value)=args.get(field){if id.is_empty()&&value.is_null()&&field!="title"{continue;}fields[field]=value.clone();}}
    if id.is_empty(){if args.get("patch").is_some()||args.get("expected").is_some(){return Err(Error::new("New projects accept fields, not an update patch or captured tokens."));}let fields=normalize(&fields,true)?;return create_normalized(state,&fields);}
    project_id(&args["id"])?;
    let patch=if let Some(patch)=args.get("patch"){if !fields.as_object().unwrap().is_empty(){return Err(Error::new("Choose either a project patch or flat project fields."));}patch}else{&fields};
    let patch=normalize(patch,false)?;update_normalized(state,id,&patch,&args["expected"])
}

#[cfg(test)]
mod tests{
    use super::*;
    use std::sync::{Arc,Barrier};
    fn fixture()->(tempfile::TempDir,Arc<Repository>,Value){let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();let created=create(&repo,&json!({"title":"Initial project","detail":"Initial detail","url":"https://example.invalid/project","cwd":temp.path(),"status":"todo"})).unwrap();(temp,repo,created)}
    fn expected(view:&Value,patch:&Value)->Value{let mut expected=json!({});for key in patch.as_object().unwrap().keys(){expected[key]=view["expected"][key].clone();}expected}
    fn patch(repo:&Repository,baseline:&Value,patch:Value)->Result<Value>{update(repo,&json!({"id":baseline["project"]["id"],"expected":expected(baseline,&patch),"patch":patch}))}
    #[test]
    fn projects_patch_preserves_latest_other_fields_unknown_metadata_and_status_override(){
        let (_temp,repo,initial)=fixture();let id=text(&initial["project"],"id");repo.transaction(|state|{state["notes"][0]["unknownNs"]=json!({"private":"project extension marker"});state["notes"][0]["archivedAt"]=json!(123);state["overrides"][id]=json!({"status":"blocked","pinned":true,"futureNs":{"preserve":true}});state["itemSettings"][id]=json!({"cwd":"unrelated mapped folder","futureNs":true});Ok(())}).unwrap();let baseline=read(&repo,&json!({"id":id})).unwrap();assert_eq!(baseline["project"]["status"],"blocked");assert!(!baseline.to_string().contains("extension marker"));
        patch(&repo,&baseline,json!({"detail":"Concurrent detail"})).unwrap();let changed=patch(&repo,&baseline,json!({"title":"New title"})).unwrap();assert_eq!(changed["changedFields"],json!(["title"]));assert_eq!(changed["project"]["detail"],"Concurrent detail");assert_eq!(changed["project"]["status"],"blocked");let state=repo.state.lock().unwrap().clone();assert_eq!(state["notes"][0]["unknownNs"]["private"],"project extension marker");assert_eq!(state["notes"][0]["archivedAt"],123);assert_eq!(state["overrides"][id]["status"],"blocked");assert_eq!(state["overrides"][id]["futureNs"]["preserve"],true);assert_eq!(state["itemSettings"][id]["cwd"],"unrelated mapped folder");
    }
    #[test]
    fn projects_status_tokens_guard_raw_and_override_and_only_explicit_status_clears_override(){
        let (_temp,repo,baseline)=fixture();let id=text(&baseline["project"],"id");repo.transaction(|s|{s["overrides"][id]=json!({"status":"blocked","hidden":true,"futureNs":7});Ok(())}).unwrap();let current=read(&repo,&json!({"id":id})).unwrap();let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();assert_eq!(patch(&repo,&baseline,json!({"status":"done"})).unwrap_err().status,409);assert_eq!(patch(&repo,&baseline,json!({"title":"Must remain initial","status":"done"})).unwrap_err().status,409);assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);
        let result=patch(&repo,&current,json!({"status":"done"})).unwrap();assert_eq!(result["project"]["status"],"done");let state=repo.state.lock().unwrap().clone();assert_eq!(state["notes"][0]["status"],"done");assert!(state["overrides"][id].get("status").is_none());assert_eq!(state["overrides"][id]["hidden"],true);assert_eq!(state["overrides"][id]["futureNs"],7);
        // A status action does not invalidate a captured token for an unchanged title.
        patch(&repo,&baseline,json!({"title":"Independent title"})).unwrap();assert_eq!(read(&repo,&json!({"id":id})).unwrap()["project"]["status"],"done");
        let baseline=read(&repo,&json!({"id":id})).unwrap();super::super::agent_access::update_work(&repo,&json!({"id":id,"status":"blocked"})).unwrap();assert_eq!(patch(&repo,&baseline,json!({"status":"todo"})).unwrap_err().status,409);
    }
    #[test]
    fn projects_same_field_race_has_one_winner_and_unrelated_field_races_both_commit(){
        for same in [true,false]{let (_temp,repo,baseline)=fixture();let gate=Arc::new(Barrier::new(3));let mut workers=vec![];for index in 0..2{let repo=repo.clone();let baseline=baseline.clone();let gate=gate.clone();workers.push(std::thread::spawn(move||{let field=if index==0||same{"title"}else{"detail"};let mut change=json!({});change[field]=json!(format!("value-{index}"));gate.wait();patch(&repo,&baseline,change)}));}gate.wait();let results=workers.into_iter().map(|worker|worker.join().unwrap()).collect::<Vec<_>>();assert_eq!(results.iter().filter(|result|result.is_ok()).count(),if same{1}else{2});if same{assert_eq!(results.into_iter().find_map(Result::err).unwrap().status,409);}else{let result=read(&repo,&json!({"id":baseline["project"]["id"]})).unwrap();assert_eq!(result["project"]["title"],"value-0");assert_eq!(result["project"]["detail"],"value-1");}assert_eq!(serde_json::from_slice::<Value>(&std::fs::read(&repo.data).unwrap()).unwrap(),*repo.state.lock().unwrap());}
    }
    #[test]
    fn projects_validate_exact_patch_tokens_and_fail_without_any_canonical_change(){
        let (_temp,repo,baseline)=fixture();let id=&baseline["project"]["id"];let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();let token=baseline["expected"]["title"].clone();
        for (patch,expected) in [(json!({}),json!({})),(json!({"title":"New"}),json!({})),(json!({"title":"New"}),json!({"title":token,"detail":baseline["expected"]["detail"]})),(json!({"title":"New"}),json!({"title":"bad"})),(json!({"title":"  "}),json!({"title":token})),(json!({"title":7}),json!({"title":token})),(json!({"title":"x".repeat(301)}),json!({"title":token})),(json!({"instruction":"run command"}),json!({"instruction":"0".repeat(64)})),(json!({"url":"https://secret:token@example.invalid"}),json!({"url":baseline["expected"]["url"]})),(json!({"cwd":"/nonexistent/project-owner-fixture-path"}),json!({"cwd":baseline["expected"]["cwd"]}))]{assert!(update(&repo,&json!({"id":id,"patch":patch,"expected":expected})).is_err());assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);}
        assert!(update(&repo,&json!({"id":"github:foreign","patch":{"title":"No upsert"},"expected":{"title":token}})).is_err());assert!(create(&repo,&json!({"title":"Client ID forbidden","id":"note:chosen"})).is_err());assert!(read(&repo,&json!({"id":id,"extra":true})).is_err());assert_eq!(*repo.state.lock().unwrap(),before);
    }
    #[test]
    fn projects_legacy_updates_need_same_tokens_and_creation_uses_same_canonical_owner(){
        let (_temp,repo,baseline)=fixture();let id=&baseline["project"]["id"];let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();assert!(repo.edit(&json!({"action":"save-note","id":id,"title":"Unchecked legacy overwrite"})).is_err());assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);
        repo.edit(&json!({"action":"save-note","id":id,"title":"Checked legacy edit","expected":{"title":baseline["expected"]["title"]}})).unwrap();let latest=read(&repo,&json!({"id":id})).unwrap();assert_eq!(latest["project"]["title"],"Checked legacy edit");assert_eq!(latest["project"]["detail"],"Initial detail");
        repo.edit(&json!({"action":"save-note","title":"Workflow project","status":"todo","cwd":null})).unwrap();let row=repo.state.lock().unwrap()["notes"][1].clone();assert!(text(&row,"id").starts_with("note:"));assert_eq!(row["cwd"],"");assert_eq!(row["kind"],"Project");let created=create(&repo,&json!({"title":"Command project"})).unwrap();assert_eq!(created["created"],true);assert_eq!(created["project"]["detail"],"");assert_eq!(created["project"]["status"],"todo");for token in created["expected"].as_object().unwrap().values(){assert_eq!(token.as_str().unwrap().len(),64);}
    }
    #[test]
    fn projects_legacy_mixed_unknown_and_missing_identity_never_bypass_checked_patch(){
        let (_temp,repo,baseline)=fixture();let id=&baseline["project"]["id"];let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();for args in [json!({"action":"save-note","id":id,"title":"Flat","patch":{"title":"Patch"},"expected":{"title":baseline["expected"]["title"]}}),json!({"action":"save-note","id":id,"instruction":"ignored executable","title":"No","expected":{"title":baseline["expected"]["title"]}}),json!({"action":"save-note","title":"Create","patch":{"title":"Unexpected update"}}),json!({"action":"save-note","title":"Create","expected":{}}),json!({"action":"save-note","id":7,"title":"No new project"}),json!({"action":"save-note","id":"note:missing","title":"No upsert","expected":{"title":baseline["expected"]["title"]}})]{assert!(repo.edit(&args).is_err());assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);}
        repo.edit(&json!({"action":"save-note","id":id,"patch":{"detail":"Checked legacy patch"},"expected":{"detail":baseline["expected"]["detail"]}})).unwrap();assert_eq!(read(&repo,&json!({"id":id})).unwrap()["project"]["detail"],"Checked legacy patch");
    }
    #[test]
    fn projects_failed_canonical_save_never_changes_metadata_or_status_override(){
        let (_temp,repo,baseline)=fixture();let id=text(&baseline["project"],"id");repo.transaction(|state|{state["overrides"][id]=json!({"status":"blocked","pinned":true});Ok(())}).unwrap();let baseline=read(&repo,&json!({"id":id})).unwrap();let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();assert_eq!(patch(&repo,&baseline,json!({"status":"done"})).unwrap_err().status,500);assert!(create(&repo,&json!({"title":"Not saved"})).is_err());assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);std::fs::remove_dir(repo.data.with_extension("tmp")).unwrap();patch(&repo,&baseline,json!({"status":"done"})).unwrap();assert_eq!(read(&repo,&json!({"id":id})).unwrap()["project"]["status"],"done");
    }

    #[test]
    fn project_folder_read_uses_launch_selector_without_metadata_cooldown_or_terminal_mutation(){
        let (temp,repo,created)=fixture();let id=text(&created["project"],"id");let root=temp.path().join("work");std::fs::create_dir(&root).unwrap();let configured=temp.path().join("configured-outside");std::fs::create_dir(&configured).unwrap();repo.transaction(|state|{state["notes"][0]["cwd"]=json!(&configured);state["notes"][0]["privateNs"]=json!("hidden-folder-projection-marker");state["itemSettings"][id]=json!({"cwd":temp.path()});Ok(())}).unwrap();let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();let revision=repo.snapshot(false)["revision"].clone();let cooldown=repo.cooldown.lock().unwrap().clone();let opening=repo.opening.lock().unwrap().clone();let running=repo.running.lock().unwrap().clone();let inventory=repo.embedded.inventory();let result=folder_with(&repo,&json!({"id":id}),&root).unwrap();assert_eq!(result,json!({"id":id,"cwd":configured.canonicalize().unwrap(),"requiresSelection":false}));assert!(!result.to_string().contains("hidden-folder-projection-marker"));assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);assert_eq!(repo.snapshot(false)["revision"],revision);assert_eq!(*repo.cooldown.lock().unwrap(),cooldown);assert_eq!(*repo.opening.lock().unwrap(),opening);assert_eq!(*repo.running.lock().unwrap(),running);assert_eq!(repo.embedded.inventory(),inventory);
        std::fs::remove_dir(&configured).unwrap();let fallback=folder_with(&repo,&json!({"id":id}),&root).unwrap();assert_eq!(fallback["cwd"],json!(temp.path().canonicalize().unwrap()));repo.transaction(|state|{state["itemSettings"]=json!({});Ok(())}).unwrap();assert_eq!(folder_with(&repo,&json!({"id":id}),&root).unwrap(),json!({"id":id,"cwd":null,"requiresSelection":true}));
    }
    #[test]
    fn project_folder_rejects_unknown_or_non_project_identity(){
        let (temp,repo,created)=fixture();let id=&created["project"]["id"];for args in [json!({"id":"github:foreign"}),json!({"id":id,"cwd":"/caller/chosen"}),json!({"id":7}),json!({})]{assert!(folder_with(&repo,&args,temp.path()).is_err());}
    }

}
