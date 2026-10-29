use super::*;
use sha2::{Digest, Sha256};
use std::path::{Component, Path};
pub fn presets(repo: &Repository, state: &Value) -> Result<Vec<Value>> {
    let entries = super::skill_presets::load(repo)?.overlay(state)?;
    let mut result = vec![];
    for entry in &entries {
        let mut v = entry.clone();
        let cwd = expand(text(entry, "cwd"));
        let supplied_skill=expand(text(entry,"skill"));
        let skill = cwd.join(&supplied_skill);
        let reports = cwd.join(entry["reports"].as_str().unwrap_or("reports"));
        let reports_path=entry["reports"].as_str().unwrap_or("reports");
        if Path::new(reports_path).is_absolute()||Path::new(reports_path).components().any(|c|c==Component::ParentDir){return Err(Error::new("Reports must stay within the preset folder."));}
        let supplied=supplied_skill.as_path();
        if supplied.components().any(|c|c==Component::ParentDir)||supplied.is_absolute()&&(skill.extension().and_then(|s|s.to_str())!=Some("md")){return Err(Error::new("Choose an existing absolute Markdown skill file, or a path within the working folder."));}
        v["cwd"] = json!(cwd);
        v["skill"] = json!(skill);
        v["reports"] = json!(reports);
        v["agent"] = json!(entry["agent"].as_str().unwrap_or("claude"));
        if !["claude", "codex"].contains(&text(&v, "agent")) {
            return Err(Error::new("Choose Claude or Codex."));
        }
        v["available"] = json!(cwd.is_dir() && skill.is_file());
        result.push(v);
    }
    Ok(result)
}
pub fn roots(repo: &Repository, state: &Value) -> Result<Vec<PathBuf>> {
    let mut roots = presets(repo, state)?
        .iter()
        .map(|p| expand(text(p, "reports")))
        .collect::<Vec<_>>();
    roots.extend(
        arr(&state["sessions"])
            .iter()
            .map(|s| expand(text(s, "cwd"))),
    );
    roots.push(repo.data.parent().unwrap().join("artifacts"));
    Ok(roots
        .into_iter()
        .filter_map(|p| p.canonicalize().ok())
        .collect())
}
pub fn kind(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_lowercase().as_str() {
        "html" => Some("text/html; charset=utf-8"),
        "md" | "txt" | "csv" | "json" | "py" => Some("text/plain; charset=utf-8"),
        "pdf" => Some("application/pdf"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}
pub fn file(path: &str, roots: &[PathBuf]) -> Result<PathBuf> {
    let path = expand(path)
        .canonicalize()
        .map_err(|_| Error::new("Artifact file unavailable."))?;
    if !roots.iter().any(|root| {
        path.strip_prefix(root).ok().is_some_and(|p| {
            !p.components()
                .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
        })
    }) || kind(&path).is_none()
        || !path.is_file()
    {
        return Err(Error::new(
            "Choose a supported artifact file inside a session or newsletter reports folder.",
        ));
    }
    if path.metadata()?.len() > 10 * 1024 * 1024 {
        return Err(Error::new("Artifacts must be 10 MiB or smaller."));
    }
    Ok(path)
}
pub fn id(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))[..24].into()
}
pub fn catalog(repo: &Repository, s: &Value) -> Result<Value> {
    let presets = presets(repo, s)?;
    let roots = roots(repo, s)?;
    let mut entries = arr(&s["artifacts"])
        .into_iter()
        .map(|x| (text(&x, "id").to_string(), x))
        .collect::<std::collections::BTreeMap<_, _>>();
    // A shared reports folder cannot identify which skill produced a file.
    // Registered artifacts carry their own provenance; discovery is only a
    // compatibility path for folders with exactly one owner.
    let mut report_owners = std::collections::BTreeMap::new();
    for preset in &presets {
        if let Ok(root) = expand(text(preset, "reports")).canonicalize() {
            *report_owners.entry(root).or_insert(0usize) += 1;
        }
    }
    for preset in &presets {
        let Ok(root) = expand(text(preset, "reports")).canonicalize() else {
            continue;
        };
        if report_owners.get(&root) != Some(&1) {
            continue;
        }
        let Ok(dirs) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut dirs = dirs
            .filter_map(|x| x.ok())
            .filter(|x| x.file_name().to_string_lossy().len() == 10 && x.path().is_dir())
            .collect::<Vec<_>>();
        dirs.sort_by_key(|x| std::cmp::Reverse(x.file_name()));
        for dir in dirs.into_iter().take(365) {
            for filename in ["report.html", "report.md", "audit.html"] {
                let Ok(path) = file(
                    &dir.path().join(filename).to_string_lossy(),
                    &[root.clone()],
                ) else {
                    continue;
                };
                let path = path.to_string_lossy().to_string();
                if entries.values().any(|entry| entry["sourcePath"] == path || entry["path"] == path) {
                    continue;
                }
                let key = id(&path);
                if entries.contains_key(&key) {
                    continue;
                }
                let title = format!(
                    "{} · {}{}",
                    text(preset, "title"),
                    dir.file_name().to_string_lossy(),
                    if filename == "audit.html" { " · Audit" } else { "" }
                );
                entries.insert(key.clone(),json!({"id":key,"title":title,"path":path,"presetId":preset["id"],"role":"skill-output","sessionIds":[],"at":std::fs::metadata(&path)?.modified()?.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64(),"format":filename.rsplit('.').next(),"discovered":true}));
            }
        }
    }
    let mut entries = entries.into_values().collect::<Vec<_>>();
    for entry in &mut entries {
        if !text(entry, "path").is_empty() {
            match file(text(entry, "path"), &roots) {
                Ok(path) => {
                    entry["available"] = json!(true);
                    entry["size"] = json!(path.metadata()?.len());
                    entry["format"] = json!(path
                        .extension()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase());
                }
                Err(_) => entry["available"] = json!(false),
            }
        } else {
            entry["available"] = json!(true);
        }
    }
    entries.sort_by(|a, b| {
        b["at"]
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&a["at"].as_f64().unwrap_or(0.0))
    });
    Ok(json!({"presets":presets,"artifacts":entries,"warnings":[]}))
}
// Metadata edits are separate from file/URL registration and preserve the
// canonical immutable identity and provenance fields without opening the file.
fn metadata_keys(value:&Value,allowed:&[&str])->Result<()>{let fields=value.as_object().ok_or(Error::new("Artifact metadata must be an object."))?;if fields.keys().any(|key|!allowed.contains(&key.as_str())){return Err(Error::new("Unknown artifact metadata field."));}Ok(())}
fn metadata_id(value:&Value)->Result<&str>{value.as_str().filter(|id|id.len()==24&&id.bytes().all(|byte|byte.is_ascii_hexdigit())).ok_or(Error::new("Supply the exact artifact ID."))}
fn metadata_conflict()->Error{let mut error=Error::new("Artifact metadata changed. Read it again and reapply only the intended changes.");error.status=409;error}
fn metadata_exact(repo:&Repository,value:&str)->Result<()>{let mut redacted=value.to_owned();commands::redact_text(repo,&mut redacted);if redacted!=value{return Err(Error::new("Artifact identity or revision cannot be safely projected."));}Ok(())}
fn metadata_links(value:&Value)->Result<Vec<String>>{let values=value.as_array().filter(|values|values.len()<=100).ok_or(Error::new("Artifact metadata supports at most 100 complete session links."))?;let mut ids=vec![];for value in values{let id=value.as_str().ok_or(Error::new("Invalid artifact session ID."))?;sid(id)?;ids.push(id.to_owned());}ids.sort();ids.dedup();Ok(ids)}
fn metadata_values(entry:&Value)->Result<Value>{let title=entry["title"].as_str().unwrap_or("Artifact");if title.chars().count()>300{return Err(Error::new("Stored artifact title exceeds its editable limit."));}let links=if entry.get("sessionIds").is_some(){metadata_links(&entry["sessionIds"])?}else{vec![]};Ok(json!({"title":title,"sessionIds":links}))}
fn metadata_token(values:&Value,field:&str,discovery:Option<&str>)->Result<String>{Ok(context_kit::sha256(&serde_json::to_vec(&json!({"value":values[field],"discovery":discovery}))?))}
fn metadata_view(repo:&Repository,entry:&Value,discovery:Option<&str>)->Result<Value>{
    let id=metadata_id(&entry["id"])?;metadata_exact(repo,id)?;let values=metadata_values(entry)?;let mut title=text(&values,"title").to_owned();commands::redact_text(repo,&mut title);if title.chars().count()>300{return Err(Error::new("Artifact title cannot be safely projected within its limit."));}
    for id in arr(&values["sessionIds"]){metadata_exact(repo,id.as_str().unwrap())?;}
    let kind=if discovery.is_some(){"discovered"}else{"registered"};metadata_exact(repo,kind)?;let mut origin=json!({"type":kind});if let Some(fingerprint)=discovery{metadata_exact(repo,fingerprint)?;origin["fingerprint"]=json!(fingerprint);}
    let mut expected=json!({});for field in ["title","sessionIds"]{let token=metadata_token(&values,field,discovery)?;metadata_exact(repo,&token)?;expected[field]=json!(token);}
    Ok(json!({"artifact":{"id":id,"title":title,"sessionIds":values["sessionIds"]},"expected":expected,"origin":origin,"scope":"local"}))
}
fn metadata_patch(args:&Value)->Result<Value>{
    metadata_keys(args,&["id","patch","expected"])?;metadata_id(&args["id"])?;metadata_keys(&args["patch"],&["title","sessionIds"])?;let patch=args["patch"].as_object().unwrap();let expected=args["expected"].as_object().ok_or(Error::new("Supply captured tokens for exactly the changed artifact fields."))?;
    if patch.is_empty()||patch.len()!=expected.len()||patch.keys().any(|key|!expected.contains_key(key))||expected.values().any(|value|!value.as_str().is_some_and(|value|value.len()==64&&value.bytes().all(|byte|byte.is_ascii_hexdigit()))){return Err(Error::new("Supply captured tokens for exactly the changed artifact fields."));}
    let mut normalized=json!({});if let Some(title)=patch.get("title"){if !title.is_string(){return Err(Error::new("Artifact title must be text."));}normalized["title"]=json!(checked(&args["patch"],"title",300,false)?);}
    if let Some(links)=patch.get("sessionIds"){normalized["sessionIds"]=json!(metadata_links(links)?);}Ok(normalized)
}
fn metadata_position(state:&Value,id:&str)->Option<usize>{state["artifacts"].as_array()?.iter().position(|entry|entry["id"]==id)}
fn metadata_apply(repo:&Repository,state:&mut Value,index:usize,patch:&Value,expected:&Value,discovery:Option<&str>)->Result<Value>{
    let entry=&state["artifacts"][index];let values=metadata_values(entry)?;let _=metadata_view(repo,entry,discovery)?;
    for field in patch.as_object().unwrap().keys(){if expected[field]!=metadata_token(&values,field,discovery)?{return Err(metadata_conflict());}metadata_exact(repo,expected[field].as_str().unwrap())?;}
    if let Some(ids)=patch.get("sessionIds"){for id in arr(ids){metadata_exact(repo,id.as_str().unwrap())?;if !arr(&values["sessionIds"]).contains(&id)&&!state["sessions"].as_array().is_some_and(|sessions|sessions.iter().any(|session|session["id"]==id)){return Err(Error::new("Choose existing sessions; retained older associations may stay unchanged."));}}}
    let mut changed=vec![];for (field,value) in patch.as_object().unwrap(){if field=="title"&&value==""{continue;}if values[field]!=*value{state["artifacts"][index][field]=value.clone();changed.push(field.clone());}}
    let mut result=metadata_view(repo,&state["artifacts"][index],None)?;result["saved"]=json!(true);result["changedFields"]=json!(changed);Ok(result)
}
fn discovery_basis(state:&Value)->Value{json!({"skillPresets":state["skillPresets"],"artifacts":state["artifacts"],"sessions":state["sessions"].as_array().map(|rows|rows.iter().map(|row|json!({"id":row["id"],"cwd":row["cwd"]})).collect::<Vec<_>>()).unwrap_or_default()})}
fn discovery_config_hash(repo:&Repository)->Result<String>{Ok(super::skill_presets::load(repo)?.hash)}
fn discovered_entry(repo:&Repository,basis:&Value,id:&str)->Result<(Value,String,String)>{let config=discovery_config_hash(repo)?;let entry=arr(&catalog(repo,basis)?["artifacts"]).into_iter().find(|entry|entry["id"]==id&&entry["discovered"]==true).ok_or_else(metadata_conflict)?;if discovery_config_hash(repo)?!=config{return Err(metadata_conflict());}let fingerprint=context_kit::sha256(&serde_json::to_vec(&json!({"entry":entry,"configRevision":config}))?);Ok((entry,fingerprint,config))}
pub fn metadata_read(repo:&Repository,args:&Value)->Result<Value>{
    metadata_keys(args,&["id"])?;let id=metadata_id(&args["id"])?;metadata_exact(repo,id)?;
    let basis={let state=repo.state.lock().map_err(|_|Error::io())?;if let Some(index)=metadata_position(&state,id){return metadata_view(repo,&state["artifacts"][index],None);}discovery_basis(&state)};
    let (entry,fingerprint,_)=discovered_entry(repo,&basis,id)?;metadata_view(repo,&entry,Some(&fingerprint))
}
pub fn metadata_update(repo:&Repository,args:&Value)->Result<Value>{let patch=metadata_patch(args)?;let id=metadata_id(&args["id"])?;metadata_exact(repo,id)?;repo.transaction_if_changed(|state|{let index=metadata_position(state,id).ok_or_else(metadata_conflict)?;metadata_apply(repo,state,index,&patch,&args["expected"],None)})}
pub fn register_discovered(repo:&Repository,args:&Value)->Result<Value>{
    let patch=metadata_patch(args)?;let id=metadata_id(&args["id"])?;metadata_exact(repo,id)?;
    let basis={let state=repo.state.lock().map_err(|_|Error::io())?;if metadata_position(&state,id).is_some(){return Err(metadata_conflict());}discovery_basis(&state)};
    let (entry,fingerprint,_)=discovered_entry(repo,&basis,id)?;metadata_view(repo,&entry,Some(&fingerprint))?;
    repo.transaction(|state|{
        if discovery_basis(state)!=basis||metadata_position(state,id).is_some(){return Err(metadata_conflict());}
        let mut stored=json!({});for field in ["id","title","path","presetId","role","sessionIds","at","format"]{if let Some(value)=entry.get(field){stored[field]=value.clone();}}
        state["artifacts"].as_array_mut().ok_or(Error::new("Artifact storage is unavailable."))?.push(stored);let index=state["artifacts"].as_array().unwrap().len()-1;
        let mut result=metadata_apply(repo,state,index,&patch,&args["expected"],Some(&fingerprint))?;result["materialized"]=json!(true);Ok(result)
    })
}
pub(super) fn legacy_metadata_update(repo:&Repository,state:&mut Value,args:&Value)->Result<Value>{metadata_keys(args,&["action","id","patch","expected"])?;let mut args=args.clone();args.as_object_mut().unwrap().remove("action");let patch=metadata_patch(&args)?;let id=metadata_id(&args["id"])?;let index=metadata_position(state,id).ok_or_else(metadata_conflict)?;metadata_apply(repo,state,index,&patch,&args["expected"],None)}
pub fn save(repo: &Repository, s: &mut Value, p: &Value) -> Result<Value> {
    if p.get("id").is_some(){return Err(Error::new("Use artifacts.metadata.read and a captured metadata.update patch for an existing artifact."));}
    let ids = p["sessionIds"].as_array().cloned().unwrap_or_default();
    if ids.len() > 100
        || ids
            .iter()
            .any(|id| !arr(&s["sessions"]).iter().any(|x| x["id"] == *id))
    {
        return Err(Error::new("Choose existing sessions."));
    }
    let mut entry = if !text(p, "path").is_empty() {
        let path = file(text(p, "path"), &roots(repo, s)?)?;
        let bytes = std::fs::read(&path)?;
        if path.extension().is_some_and(|x| x == "json") {
            validate_table(&serde_json::from_slice(&bytes)?)?;
        }
        let extension = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        let key = id(&format!(
            "{}:{extension}:{:x}",
            path.to_string_lossy(),
            Sha256::digest(&bytes)
        ));
        let directory = repo.data.parent().unwrap().join("artifacts");
        std::fs::create_dir_all(&directory)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
        let stored = directory.join(format!("{key}.{extension}"));
        if !stored.exists() {
            super::http::private_write(&stored, &bytes)?;
        }
        json!({"id":key,"path":stored,"sourcePath":path,"at":now(),"format":extension,"snapshot":true})
    } else {
        let url = safe_url(text(p, "url"))?;
        if !url.starts_with("https://") {
            return Err(Error::new("Enter an HTTPS artifact URL."));
        }
        json!({"id":id(&url),"url":url,"at":now(),"format":"link"})
    };
    let old = arr(&s["artifacts"])
        .into_iter()
        .find(|x| x["id"] == entry["id"])
        .unwrap_or(json!({}));
    let existing=old.is_object()&&!old.as_object().unwrap().is_empty();
    if existing {
        metadata_exact(repo,text(&old,"id"))?;
        let supplied_title=checked(p,"title",300,false)?;
        if (!supplied_title.is_empty()&&supplied_title!=text(&old,"title"))||(!text(p,"presetId").is_empty()&&p["presetId"]!=old["presetId"]){let mut error=metadata_conflict();error.message="This artifact already exists. Open its metadata editor to change the title or session links. Existing provenance cannot be reassigned.".into();error.data=json!({"artifactId":old["id"]});return Err(error);}
        entry=old.clone();
    }
    let title = if !text(p, "title").trim().is_empty() {
        checked(p, "title", 300, true)?
    } else {
        old["title"]
            .as_str()
            .or(entry["title"].as_str())
            .unwrap_or_else(|| {
                if !text(&entry, "path").is_empty() {
                    "Report"
                } else {
                    "Artifact"
                }
            })
            .to_string()
    };
    let mut links = if p["replaceSessions"] == true && !existing {
        vec![]
    } else {
        arr(&old["sessionIds"])
    };
    links.extend(ids);
    links.sort_by_key(Value::to_string);
    links.dedup();
    if links.len()>100{return Err(Error::new("Keep at most 100 complete session links per artifact."));}
    entry["sessionIds"] = json!(links);
    entry["title"] = json!(title);
    if !text(p,"presetId").is_empty()&&!existing{
        if !presets(repo,s)?.iter().any(|preset|preset["id"]==p["presetId"]){return Err(Error::new("Choose an existing skill for this output."));}
        entry["presetId"]=p["presetId"].clone();entry["role"]=json!("skill-output");
    } else {
        // Reattaching an immutable snapshot updates its links/title, not its
        // previously registered ownership.
        for field in ["presetId", "role"] {
            if let Some(value) = old.get(field) {
                entry[field] = value.clone();
            }
        }
    }
    let mut entries = arr(&s["artifacts"]);
    entries.retain(|x| x["id"] != entry["id"]);
    entries.push(entry.clone());
    s["artifacts"] = json!(entries);
    Ok(entry)
}
/// Register a file snapshot or HTTPS link for one existing chat. Both the UI
/// and native helper use this owner so skill attribution is inferred identically.
pub fn attach_to_chat(repo:&Repository,record_id:&str,input:&Value)->Result<Value>{
    let record_id=sid(record_id)?;
    for key in ["path","url","title"] {if input.get(key).is_some_and(|value|!value.is_string()){return Err(Error::new(format!("Invalid {key}.")));}}
    if input.get("skillOutput").is_some_and(|value|!value.is_boolean()){return Err(Error::new("skillOutput must be true or false."));}
    let path=checked(input,"path",4096,false)?;let url=checked(input,"url",4096,false)?;let title=checked(input,"title",300,false)?;
    if path.is_empty()==url.is_empty(){return Err(Error::new("Choose exactly one artifact file or HTTPS URL."));}
    repo.transaction(|state|{
        let record=arr(&state["sessions"]).into_iter().find(|record|record["id"]==record_id).ok_or(Error::new("Session record no longer exists."))?;
        let mut payload=json!({"path":path,"url":url,"title":title,"sessionIds":[record_id]});
        if input["skillOutput"]==true {
            if text(&record,"newsletterId").is_empty(){return Err(Error::new("This chat has no assigned skill. Attach the artifact without --skill-output."));}
            payload["presetId"]=record["newsletterId"].clone();
        }
        let entry=save(repo,state,&payload)?;
        Ok(json!({"artifactId":entry["id"],"recordId":record_id,"saved":true,"scope":"local",
            "role":if entry["role"]=="skill-output"{"skill-output"}else{"attachment"},
            "presetId":entry["presetId"].as_str().filter(|id|id.len()<=80),"snapshot":entry["snapshot"]==true,"format":entry["format"]}))
    })
}
pub fn read(repo: &Repository, id: &str) -> Result<(Vec<u8>, String)> {
    if id.len() != 24 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::new("Invalid artifact ID."));
    }
    let s = repo.state.lock().unwrap();
    let entry = arr(&catalog(repo, &s)?["artifacts"])
        .into_iter()
        .find(|x| x["id"] == id)
        .ok_or(Error::new("Artifact unavailable."))?;
    let path = file(text(&entry, "path"), &roots(repo, &s)?)?;
    let kind = kind(&path).unwrap().into();
    let bytes = std::fs::read(path)?;
    if bytes.len() > 10 * 1024 * 1024 {
        return Err(Error::new("Artifact exceeds 10 MiB."));
    }
    if entry["format"] == "json" {
        validate_table(&serde_json::from_slice(&bytes)?)?;
    }
    Ok((bytes, kind))
}
pub fn validate_table(value: &Value) -> Result<()> {
    let columns = arr(&value["columns"]);
    let rows = arr(&value["rows"]);
    if value["schema"] != "mydash.table/v1"
        || columns.is_empty()
        || columns.len() > 100
        || columns.iter().any(|x| !x.is_string())
        || !value["rows"].is_array()
        || rows.len() > 10000
        || rows.iter().any(|x| {
            !x.is_array()
                || arr(x).len() != columns.len()
                || arr(x)
                    .iter()
                    .any(|cell| cell.is_object() || cell.is_array())
        })
    {
        return Err(Error::new(
            "Use mydash.table/v1 JSON with columns and up to 10,000 equal-width rows.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod attachment_tests {
    use super::*;
    fn fixture()->(tempfile::TempDir,std::sync::Arc<Repository>,String,String,PathBuf){
        let temp=tempfile::tempdir().unwrap();let cwd=temp.path().join("project");std::fs::create_dir_all(cwd.join("reports")).unwrap();std::fs::write(cwd.join("skill.md"),"Fixture skill").unwrap();
        let file=cwd.join("reports/report.html");std::fs::write(&file,"<p>Fixture result</p>").unwrap();let first=uuid();let second=uuid();let repo=Repository::open(temp.path().into(),None,true).unwrap();
        repo.transaction(|state|{state["sessions"]=json!([{"id":first,"cwd":cwd,"items":[],"newsletterId":"first"},{"id":second,"cwd":cwd,"items":[],"newsletterId":"second"}]);state["skillPresets"]=json!([{"id":"first","cwd":cwd,"skill":"skill.md","reports":"reports"},{"id":"second","cwd":cwd,"skill":"skill.md","reports":"reports"}]);Ok(())}).unwrap();(temp,repo,first,second,file)
    }
    #[test]
    fn attachment_identity_and_explicit_skill_provenance_survive_reattachment_and_legacy_cli(){
        let (_temp,repo,first,second,file)=fixture();
        let result=attach_to_chat(&repo,&first,&json!({"path":file,"title":"Produced report","skillOutput":true})).unwrap();assert_eq!(result["role"],"skill-output");assert_eq!(result["presetId"],"first");assert_eq!(result["snapshot"],true);assert_eq!(result.as_object().unwrap().len(),8);
        assert!(!result.to_string().contains("sourcePath"));assert!(!result.to_string().contains("project/reports"));
        assert_eq!(attach_to_chat(&repo,&second,&json!({"path":file,"title":"Reattached"})).unwrap_err().status,409);let again=attach_to_chat(&repo,&second,&json!({"path":file})).unwrap();assert_eq!(result["artifactId"],again["artifactId"]);assert_eq!(again["presetId"],"first");assert_eq!(again["role"],"skill-output");
        let registration=super::super::workflows::register(&repo,&json!({"operation":"save-artifact","id":second,"path":file})).unwrap();let reply=super::super::workflows::registration_reply(&registration);assert_eq!(reply["id"],second);assert_eq!(reply["artifactReceipt"]["artifactId"],result["artifactId"]);assert_eq!(reply["artifactReceipt"]["presetId"],"first");
        let state=repo.state.lock().unwrap();assert_eq!(arr(&state["artifacts"]).len(),1);let entry=&state["artifacts"][0];assert_eq!(arr(&entry["sessionIds"]).len(),2);assert_eq!(std::fs::read(text(entry,"path")).unwrap(),b"<p>Fixture result</p>");
    }
    #[test]
    fn attachment_urls_do_not_fetch_and_owner_rejects_invalid_files_targets_and_provenance(){
        let (temp,repo,first,_,file)=fixture();let link=attach_to_chat(&repo,&first,&json!({"url":"https://example.invalid/result"})).unwrap();assert_eq!(link["format"],"link");assert_eq!(link["snapshot"],false);assert_eq!(link["presetId"],Value::Null);assert_eq!(link["role"],"attachment");
        let outside=temp.path().join("outside.md");std::fs::write(&outside,"not in a registered root").unwrap();let hidden=file.parent().unwrap().join(".private.md");std::fs::write(&hidden,"private").unwrap();let oversized=file.parent().unwrap().join("large.md");std::fs::File::create(&oversized).unwrap().set_len(10*1024*1024+1).unwrap();
        let plain=uuid();repo.transaction(|state|{state["sessions"].as_array_mut().unwrap().push(json!({"id":plain,"cwd":file.parent().unwrap(),"items":[]}));Ok(())}).unwrap();
        let before=repo.state.lock().unwrap().clone();let disk=std::fs::read(&repo.data).unwrap();
        for (id,input) in [(first.clone(),json!({"path":file,"url":"https://example.invalid/result"})),(first.clone(),json!({"url":"http://example.invalid/result"})),(first.clone(),json!({"url":"https://user:secret@example.invalid/result"})),(first.clone(),json!({"path":outside})),(first.clone(),json!({"path":hidden})),(first.clone(),json!({"path":oversized})),(first.clone(),json!({"path":file,"skillOutput":"true"})),(plain,json!({"path":file,"skillOutput":true})),(uuid(),json!({"path":file}))]{assert!(attach_to_chat(&repo,&id,&input).is_err());assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),disk);}
    }
    #[test]
    fn failed_attachment_metadata_commit_returns_no_saved_receipt_or_state_change(){
        let (_temp,repo,first,_,file)=fixture();let before=repo.state.lock().unwrap().clone();let disk=std::fs::read(&repo.data).unwrap();std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();
        assert!(attach_to_chat(&repo,&first,&json!({"path":file})).is_err());assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),disk);
    }
}

#[cfg(test)]
mod metadata_tests {
 use super::*;use std::sync::Arc;
 fn fixture()->(tempfile::TempDir,Arc<Repository>,String,Vec<String>){let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();let ids=(0..32).map(|_|uuid()).collect::<Vec<_>>();let url="https://example.invalid/metadata-report";let artifact=id(url);repo.transaction(|state|{state["sessions"]=json!(ids.iter().map(|id|json!({"id":id,"cwd":temp.path(),"items":[]})).collect::<Vec<_>>());state["artifacts"]=json!([{"id":artifact,"title":"Original title","sessionIds":ids[..31],"url":url,"format":"link","at":42,"snapshot":true,"presetId":"original-owner","role":"skill-output","privateNs":{"keep":"private-metadata-marker"}}]);Ok(())}).unwrap();(temp,repo,artifact,ids)}
 fn update(repo:&Repository,read:&Value,patch:Value)->Result<Value>{let mut expected=json!({});for key in patch.as_object().unwrap().keys(){expected[key]=read["expected"][key].clone();}metadata_update(repo,&json!({"id":read["artifact"]["id"],"patch":patch,"expected":expected}))}
 #[test]fn artifact_metadata_full_read_is_pure_registered_metadata_and_preserves_more_than_twenty_links(){let (temp,repo,id,ids)=fixture();let config=repo.data.parent().unwrap().join("config/skills.json");std::fs::create_dir_all(config.parent().unwrap()).unwrap();std::fs::write(config,b"invalid catalog config").unwrap();let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();let result=metadata_read(&repo,&json!({"id":id})).unwrap();assert_eq!(result["origin"],json!({"type":"registered"}));assert_eq!(result["artifact"]["sessionIds"].as_array().unwrap().len(),31);let mut sorted=ids[..31].to_vec();sorted.sort();assert_eq!(result["artifact"]["sessionIds"],json!(sorted));assert!(!result.to_string().contains("private-metadata-marker"));assert!(!result.to_string().contains("example.invalid"));assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);assert!(repo.embedded.inventory().is_empty());}
 #[test]fn artifact_metadata_field_cas_commutes_and_preserves_immutable_data_and_extensions(){let (_temp,repo,id,ids)=fixture();let baseline=metadata_read(&repo,&json!({"id":id})).unwrap();let links=update(&repo,&baseline,json!({"sessionIds":ids})).unwrap();let renamed=update(&repo,&baseline,json!({"title":"New title"})).unwrap();assert_eq!(renamed["artifact"]["sessionIds"].as_array().unwrap().len(),32);assert_eq!(renamed["changedFields"],json!(["title"]));assert_eq!(update(&repo,&baseline,json!({"title":"Stale title"})).unwrap_err().status,409);let final_links=update(&repo,&links,json!({"sessionIds":[]})).unwrap();assert_eq!(final_links["artifact"]["title"],"New title");let state=repo.state.lock().unwrap();let artifact=&state["artifacts"][0];assert_eq!(artifact["presetId"],"original-owner");assert_eq!(artifact["role"],"skill-output");assert_eq!(artifact["snapshot"],true);assert_eq!(artifact["at"],42);assert_eq!(artifact["privateNs"]["keep"],"private-metadata-marker");assert_eq!(artifact["url"],"https://example.invalid/metadata-report");}
 #[test]fn artifact_metadata_conflicts_invalid_patch_and_unchecked_legacy_save_leave_canonical_unchanged(){let (_temp,repo,id,ids)=fixture();let read=metadata_read(&repo,&json!({"id":id})).unwrap();update(&repo,&read,json!({"sessionIds":[ids[0]]})).unwrap();let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();assert_eq!(update(&repo,&read,json!({"sessionIds":[]})).unwrap_err().status,409);for args in [json!({"id":id,"patch":{"title":"New"},"expected":{}}),json!({"id":id,"patch":{"title":"New"},"expected":read["expected"]}),json!({"id":id,"patch":{"path":"/private/changed"},"expected":{"path":"0".repeat(64)}}),json!({"id":id,"patch":{"sessionIds":[uuid()]},"expected":{"sessionIds":metadata_read(&repo,&json!({"id":id})).unwrap()["expected"]["sessionIds"]}})]{assert!(metadata_update(&repo,&args).is_err());}assert!(repo.edit(&json!({"action":"save-artifact","id":id,"title":"Unchecked","sessionIds":[],"replaceSessions":true})).is_err());assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);repo.edit(&json!({"action":"save-artifact","id":id,"patch":{"title":"Checked legacy"},"expected":{"title":read["expected"]["title"]}})).unwrap();assert_eq!(metadata_read(&repo,&json!({"id":id})).unwrap()["artifact"]["title"],"Checked legacy");}
 fn discovery_fixture()->(tempfile::TempDir,Arc<Repository>,String,std::path::PathBuf){let temp=tempfile::tempdir().unwrap();let cwd=temp.path().join("skill");let dir=cwd.join("reports/2026-10-27");std::fs::create_dir_all(&dir).unwrap();std::fs::write(cwd.join("SKILL.md"),"Fixture skill").unwrap();let path=dir.join("report.md");std::fs::write(&path,"Initial report").unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();repo.transaction(|state|{state["skillPresets"]=json!([{"id":"fixture-skill","title":"Fixture skill","cwd":cwd,"skill":"SKILL.md","reports":"reports"}]);Ok(())}).unwrap();let artifact=id(&path.canonicalize().unwrap().to_string_lossy());(temp,repo,artifact,path)}
 #[test]fn artifact_metadata_discovery_read_does_not_register_and_materialization_checks_current_fingerprint(){let (_temp,repo,id,path)=discovery_fixture();let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();let read=metadata_read(&repo,&json!({"id":id})).unwrap();assert_eq!(read["origin"]["type"],"discovered");assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);assert!(update(&repo,&read,json!({"title":"No silent registration"})).is_err());std::fs::write(&path,"Changed report with a different size").unwrap();let args=json!({"id":id,"patch":{"sessionIds":[]},"expected":{"sessionIds":read["expected"]["sessionIds"]}});assert_eq!(register_discovered(&repo,&args).unwrap_err().status,409);assert_eq!(*repo.state.lock().unwrap(),before);let fresh=metadata_read(&repo,&json!({"id":id})).unwrap();let result=register_discovered(&repo,&json!({"id":id,"patch":{"title":"Registered title"},"expected":{"title":fresh["expected"]["title"]}})).unwrap();assert_eq!(result["materialized"],true);assert_eq!(result["origin"]["type"],"registered");let state=repo.state.lock().unwrap();let stored=&state["artifacts"][0];assert_eq!(stored["presetId"],"fixture-skill");assert_eq!(stored["role"],"skill-output");for key in ["available","size","discovered"]{assert!(stored.get(key).is_none());}assert_eq!(stored["path"],json!(path.canonicalize().unwrap()));}
 #[test]fn artifact_metadata_failed_save_and_reattachment_preserve_extensions_and_latest_metadata(){let (_temp,repo,id,ids)=fixture();let read=metadata_read(&repo,&json!({"id":id})).unwrap();let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();assert!(update(&repo,&read,json!({"title":"Not committed"})).is_err());assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);std::fs::remove_dir(repo.data.with_extension("tmp")).unwrap();update(&repo,&read,json!({"title":"Current title"})).unwrap();attach_to_chat(&repo,&ids[31],&json!({"url":"https://example.invalid/metadata-report"})).unwrap();let state=repo.state.lock().unwrap();assert_eq!(state["artifacts"][0]["title"],"Current title");assert_eq!(state["artifacts"][0]["privateNs"]["keep"],"private-metadata-marker");assert_eq!(state["artifacts"][0]["at"],42);assert_eq!(state["artifacts"][0]["sessionIds"].as_array().unwrap().len(),32);}
 #[test]fn artifact_metadata_actionable_ids_and_tokens_are_exact_or_fail_before_any_write(){
  for mode in ["artifact-id","session-id","field-token"]{let temp=tempfile::tempdir().unwrap();let artifact=id("https://example.invalid/safe");let session=uuid();let values=json!({"title":"Original","sessionIds":[session]});let token=metadata_token(&values,"title",None).unwrap();let secret=match mode{"artifact-id"=>artifact.as_str(),"session-id"=>session.as_str(),_=>token.as_str()};std::fs::write(temp.path().join(".env"),format!("MYDASH_TEST_SECRET={secret}\n")).unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();repo.transaction(|state|{state["sessions"]=json!([{"id":session}]);state["artifacts"]=json!([{"id":artifact,"title":"Original","sessionIds":[session],"url":"https://example.invalid/safe"}]);Ok(())}).unwrap();let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();let error=commands::invoke(&repo,&json!({"name":"artifacts.metadata.read","args":{"id":artifact}}),true).unwrap_err();assert!(!error.message.contains(secret));assert!(commands::invoke(&repo,&json!({"name":"artifacts.metadata.update","args":{"id":artifact,"patch":{"title":"Changed"},"expected":{"title":token}}}),true).is_err());assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);}
 }

 #[test]fn artifact_metadata_duplicate_no_id_registration_is_additive_and_cannot_rename_or_reassign_provenance(){let (_temp,repo,id,ids)=fixture();let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();for extra in [json!({"title":"Different title"}),json!({"presetId":"other-owner"})]{let mut input=json!({"url":"https://example.invalid/metadata-report","sessionIds":[ids[31]],"replaceSessions":true});for (key,value) in extra.as_object().unwrap(){input[key]=value.clone();}let error=repo.transaction(|state|save(&repo,state,&input)).unwrap_err();assert_eq!(error.status,409);assert_eq!(error.data,json!({"artifactId":id}));assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);}let result=repo.transaction(|state|save(&repo,state,&json!({"url":"https://example.invalid/metadata-report","title":"Original title","presetId":"original-owner","sessionIds":[ids[31]],"replaceSessions":true}))).unwrap();assert_eq!(result["sessionIds"].as_array().unwrap().len(),32);assert_eq!(result["title"],"Original title");assert_eq!(result["presetId"],"original-owner");assert_eq!(result["role"],"skill-output");assert_eq!(result["at"],42);assert_eq!(result["privateNs"]["keep"],"private-metadata-marker");}
 #[test]fn artifact_metadata_discovery_configuration_revision_invalidates_old_tokens_without_materializing(){let (_temp,repo,id,_path)=discovery_fixture();let read=metadata_read(&repo,&json!({"id":id})).unwrap();let path=repo.data.parent().unwrap().join("config/skills.json");let mut config=super::super::config::read(&path).unwrap();config["privateRevisionCanary"]=json!("changed configuration");std::fs::write(path,serde_json::to_vec(&config).unwrap()).unwrap();let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();let result=register_discovered(&repo,&json!({"id":id,"patch":{"sessionIds":[]},"expected":{"sessionIds":read["expected"]["sessionIds"]}}));assert_eq!(result.unwrap_err().status,409);assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);}

 #[test]fn artifact_metadata_complete_link_limit_applies_to_incremental_reattachment(){let (_temp,repo,id,_)=fixture();let ids=(0..101).map(|_|uuid()).collect::<Vec<_>>();repo.transaction(|state|{state["sessions"]=json!(ids.iter().map(|id|json!({"id":id})).collect::<Vec<_>>());state["artifacts"][0]["sessionIds"]=json!(ids[..100]);Ok(())}).unwrap();assert_eq!(metadata_read(&repo,&json!({"id":id})).unwrap()["artifact"]["sessionIds"].as_array().unwrap().len(),100);let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();assert!(attach_to_chat(&repo,&ids[100],&json!({"url":"https://example.invalid/metadata-report"})).is_err());assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);}

}
