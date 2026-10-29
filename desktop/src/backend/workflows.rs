use super::*;
use std::sync::Arc;
use super::{artifacts, domain, terminal};
use super::chat_preparation::{self, Request};
pub fn work_root() -> PathBuf {
    std::env::var("MYDASH_WORKDIR")
        .map(|s| expand(&s))
        .unwrap_or_else(|_| home().join("code"))
}
pub fn folders() -> Value {folders_in(&work_root())}
fn folders_in(root:&std::path::Path)->Value{
    let mut result = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| {
            e.path()
                .canonicalize()
                .ok()
                .map(|p| json!({"name":e.file_name().to_string_lossy(),"path":p}))
        })
        .collect::<Vec<_>>();
    result.sort_by_key(|x| text(x, "name").to_lowercase());
    json!(result)
}
fn repo_name(item: &Value) -> String {
    reqwest::Url::parse(text(item, "url"))
        .ok()
        .map(|u| {
            u.path()
                .trim_matches('/')
                .split('/')
                .take(2)
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_default()
}
pub fn contexts(item: &Value) -> Vec<Value> {
    let mut result = vec![];
    match text(item,"source"){"github"=>{let name=repo_name(item);result.push(json!({"key":format!("github:{}",name.to_lowercase()),"label":format!("GitHub repo {name}")}));},"linear"=>{let mut parent=&item["parent"];let mut top=Value::Null;for _ in 0..8 {if parent.is_null()||parent.as_object().is_none(){break;}top=parent.clone();parent=&parent["parent"];}if top.is_null()&&!arr(&item["children"]).is_empty(){top=item.clone();}if !top.is_null(){result.push(json!({"key":format!("linear-family:{}",text(&top,"id")),"label":format!("Issue family {}",text(&top,"reference"))}));}if !text(item,"projectId").is_empty(){let workspace=text(item,"url").split('/').nth(3).unwrap_or("");result.push(json!({"key":format!("linear-project:{workspace}:{}",text(item,"projectId")),"label":format!("Linear project {}",text(item,"project"))}));}},"sentry"=>result.push(json!({"key":format!("sentry-project:{}",text(item,"project")),"label":format!("Sentry project {}",text(item,"project"))})),_=>()};
    if !text(item, "id").is_empty() {
        result
            .push(json!({"key":format!("item:{}",text(item,"id")),"label":"Only this work item"}));
    }
    result
}
/// Authoritative automatic choice, shared by launch and the read-only project resolver.
/// A saved path is an existing-directory hint; it never creates a folder or changes mappings.
pub(super) fn automatic_folder(s:&Value,item:&Value,root:&std::path::Path,candidates:&Value)->Option<String>{
    let root=root.canonicalize().unwrap_or_else(|_|root.to_path_buf());
    let usable=|value:&str|directory(value).ok().filter(|path|PathBuf::from(path)!=root);
    let setting=&s["itemSettings"][text(item,"id")];
    if setting["explicit"]==true{if let Some(path)=usable(text(setting,"cwd")){return Some(path);}}
    for choice in contexts(item){if let Some(path)=usable(text(&s["folderMappings"][text(&choice,"key")],"path")){return Some(path);}}
    if item["source"]=="notes"{if let Some(path)=usable(text(item,"cwd")){return Some(path);}}
    if item["source"]=="github"{
        let name=repo_name(item).split('/').last().unwrap_or("").to_lowercase();
        if let Some(path)=arr(candidates).iter().filter(|folder|text(folder,"name").to_lowercase()==name).find_map(|folder|usable(text(folder,"path"))){return Some(path);}
    }
    usable(text(setting,"cwd"))
}
fn choose_with(s:&Value,item:&Value,p:&Value,root:&std::path::Path,candidates:&Value)->Result<(String,Option<String>)>{
    let choices = contexts(item);
    let manual = text(p, "folderPath");
    let new = checked(p, "newFolder", 80, false)?;
    if !new.is_empty() {
        if !new
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
            || !new.chars().next().unwrap().is_ascii_alphanumeric()
        {
            return Err(Error::new("Use a simple folder name without paths."));
        }
        let path = root.join(&new);
        if path.exists() {
            return Err(Error::new("That folder already exists."));
        }
        return Ok((path.to_string_lossy().into(), Some(new)));
    }
    if !manual.is_empty() {
        if !arr(&candidates).iter().any(|x| x["path"] == manual) {
            return Err(Error::new(
                "Choose an existing folder under the work directory.",
            ));
        }
        return Ok((directory(manual)?, None));
    }
    if p["chooseFolder"]!=true{
        if let Some(path)=automatic_folder(s,item,root,candidates){return Ok((path,None));}
        if item.is_null()&&root.is_dir(){return Ok((root.to_string_lossy().into(),None));}
    }
    let mut error = Error::new("Choose a folder for this session.");
    error.status = 409;
    error.data = json!({"location":{"root":root,"folders":candidates,"contexts":choices,"suggestedName":repo_name(item).split('/').last().unwrap_or(""),"title":item["title"].as_str().unwrap_or("New session"),"reason":"Choose where the agent should work. This can be remembered for related work."}});
    Err(error)
}
fn session_text(fields:&Value)->Result<(String,String)>{
    let title=checked(fields,"title",180,true)?;
    if title.chars().any(char::is_control){return Err(Error::new("Session names cannot contain control characters."));}
    Ok((title,checked(fields,"instruction",8000,false)?))
}
#[derive(Clone,Copy,PartialEq)]
enum SaveIntent{Replace,Register}
pub fn save_session(s: &mut Value, p: &Value) -> Result<Value> {save_session_with_intent(s,p,SaveIntent::Replace)}
fn save_session_with_intent(s:&mut Value,p:&Value,intent:SaveIntent)->Result<Value>{
    let mut sessions = arr(&s["sessions"]);
    let by_id = if text(p, "id").is_empty() {
        sessions.iter().position(|x| {
            !text(p, "sessionId").is_empty()
                && x["sessionId"] == p["sessionId"]
                && x["agent"] == p["agent"]
        })
    } else {
        sessions.iter().position(|x| x["id"] == p["id"])
    };
    if !text(p, "id").is_empty() && by_id.is_none() {
        return Err(Error::new("Session association no longer exists."));
    }
    let old = by_id.map(|i| sessions[i].clone()).unwrap_or(json!({}));
    let mut merged = old.clone();
    if !merged.is_object() {
        merged = json!({});
    }
    for (k, v) in p
        .as_object()
        .ok_or(Error::new("Expected a session object."))?
    {
        merged[k] = v.clone();
    }
    if intent==SaveIntent::Register&&text(p,"title").trim().is_empty(){merged["title"]=if old.is_object()&&!text(&old,"title").is_empty(){old["title"].clone()}else{json!("Agent session")};}
    let agent = text(&merged, "agent");
    if !["claude", "codex"].contains(&agent) {
        return Err(Error::new("Choose Claude or Codex."));
    }
    let (title,instruction)=session_text(&merged)?;
    let cwd=directory(text(&merged,"cwd"))?;
    let destination = match merged["destination"].as_str().unwrap_or("embedded") {
        "cmux" => "embedded",
        other => other,
    };
    if !["embedded", "desktop"].contains(&destination)
        || (agent == "claude" && destination == "desktop")
    {
        return Err(Error::new("Choose the embedded terminal or Codex desktop."));
    }
    let mut ids = p["itemIds"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| arr(&old["items"]).iter().map(|x| x["id"].clone()).collect());
    if (intent==SaveIntent::Register||text(p,"id").is_empty()) && !old.is_null() {
        ids.extend(arr(&old["items"]).iter().map(|x| x["id"].clone()));
    }
    ids.sort_by_key(Value::to_string);
    ids.dedup();
    if ids.len() > 100 || ids.iter().any(|x| !x.is_string()) {
        return Err(Error::new("Choose up to 100 linked work items."));
    }
    let catalog = domain::catalog(s);
    let mut items = if intent==SaveIntent::Register{arr(&old["items"])}else{vec![]};
    for id in ids {
        if intent==SaveIntent::Register&&items.iter().any(|item|item["id"]==id){continue;}
        let item = catalog
            .get(id.as_str().unwrap())
            .ok_or(Error::new("A linked work item no longer exists."))?;
        items.push(domain::item_ref(item));
        if !text(item, "detail").is_empty() {
            s["itemArchive"][text(item, "id")] = item.clone();
        }
        if !s["itemSettings"][text(item, "id")].is_object() {
            s["itemSettings"][text(item, "id")] = json!({});
        }
        if p["preserveItemFolders"] != true
            || text(&s["itemSettings"][text(item, "id")], "cwd").is_empty()
        {
            s["itemSettings"][text(item, "id")]["cwd"] = json!(cwd);
        }
    }
    let mut sid_value = merged["sessionId"]
        .as_str()
        .filter(|x| !x.is_empty())
        .map(sid)
        .transpose()?;
    if !text(&old, "sessionId").is_empty()
        && (sid_value.as_deref() != Some(text(&old, "sessionId")) || old["agent"] != agent)
    {
        return Err(Error::new(
            "Remove the association before changing its conversation identity.",
        ));
    }
    let mut launch = old["launchNew"].as_bool().unwrap_or(sid_value.is_none());
    if sid_value.is_some() && text(&old, "sessionId").is_empty() {
        launch = false;
    }
    if by_id.is_none() && sid_value.is_none() {
        launch = true;
        if agent == "claude" {
            sid_value = Some(uuid());
        }
    }
    let id = old["id"].as_str().map(str::to_owned).unwrap_or_else(uuid);
    let mut session = old.clone();
    for(k,v)in json!({"id":id,"agent":agent,"title":title,"cwd":cwd,"sessionId":sid_value,"destination":destination,"instruction":instruction,"items":items,"launchNew":launch,"hidden":old["hidden"].as_bool().unwrap_or(false),"archivedAt":old["archivedAt"],"createdAt":old["createdAt"].as_f64().unwrap_or_else(now),"updatedAt":now()}).as_object().unwrap(){session[k]=v.clone();}
    sessions.retain(|x| x["id"] != id);
    sessions.push(session.clone());
    s["sessions"] = json!(sessions);
    Ok(session)
}
#[derive(Clone,Copy,PartialEq)]
enum PrepareIntent{LegacyForm,Internal,Register}
pub fn prepare(repo:&Repository,p:&Value)->Result<Value>{if !text(p,"id").is_empty(){return super::chat_edit::legacy(repo,p);}{let state=repo.state.lock().map_err(|_|Error::io())?;super::chat_edit::reject_unchecked_association(&state,p)?;}prepare_impl(repo,p,None,PrepareIntent::LegacyForm)}
pub(super) fn prepare_with_request(repo: &Repository, p: &Value, request:Option<&Request>) -> Result<Value> {prepare_impl(repo,p,request,PrepareIntent::Internal)}
fn prepare_impl(repo: &Repository,p:&Value,request:Option<&Request>,intent:PrepareIntent)->Result<Value>{prepare_resolving(repo,p,request,intent,|url|repo.providers.resolve(url))}
fn prepare_resolving(repo:&Repository,p:&Value,request:Option<&Request>,intent:PrepareIntent,resolve:impl Fn(&str)->Result<Value>)->Result<Value>{
    let mut imported = vec![];
    let urls = arr(&p["urls"]);
    if urls.len() > 10 {
        return Err(Error::new("Import at most 10 links."));
    }
    for url in urls {
        let cached = {
            let s = repo.state.lock().unwrap();
            domain::catalog(&s).into_values().find(|x| x["url"] == url)
        };
        imported.push(match cached {
            Some(v) => v,
            None => resolve(url.as_str().ok_or(Error::new("Invalid source URL."))?)?,
        });
    }
    repo.transaction(|s| {
        if intent==PrepareIntent::LegacyForm{super::chat_edit::reject_unchecked_association(s,p)?;}
        if let Some(existing)=chat_preparation::lookup(repo,request,s)?{return Ok(existing)}
        if request.is_some_and(|request|request.kind=="attach") {if let Some(existing)=arr(&s["sessions"]).into_iter().find(|row|row["agent"]==p["agent"]&&row["sessionId"]==p["sessionId"]){return chat_preparation::finish(repo,request,s,existing,true);}}

        let mut payload = p.clone();
        let mut ids = arr(&p["itemIds"]);
        for item in &imported {
            if intent==PrepareIntent::Register&&domain::catalog(s).contains_key(text(item,"id")){ids.push(item["id"].clone());continue;}
            let mut items = arr(&s["items"]);
            items.retain(|x| x["id"] != item["id"]);
            items.push(item.clone());
            s["items"] = json!(items);
            ids.push(item["id"].clone());
        }
        let name = p["projectName"]
            .as_str()
            .or(p["project"].as_str())
            .unwrap_or("");
        if !name.trim().is_empty() {
            let project = arr(&s["notes"])
                .into_iter()
                .find(|x| x["title"] == name && x["archivedAt"].is_null());
            let id = if let Some(project) = project {
                text(&project, "id").to_owned()
            } else {
                domain::edit(
                    s,
                    &json!({"action":"save-note","title":name,"status":"todo","cwd":p["cwd"]}),
                )?;
                text(arr(&s["notes"]).last().unwrap(), "id").to_owned()
            };
            ids.push(json!(id));
        }
        payload["itemIds"] = json!(ids);
        if text(&payload,"title").is_empty()&&intent!=PrepareIntent::Register {
            payload["title"] = json!(if name.is_empty() {
                "Agent session"
            } else {
                name
            });
        }
        let record=save_session_with_intent(s,&payload,if intent==PrepareIntent::Register{SaveIntent::Register}else{SaveIntent::Replace})?;chat_preparation::finish(repo,request,s,record,false)
    })
}
// Best-effort rollback only: a crash can leave a folder, and inode checking is
// not atomic with rmdir against a concurrent rename. Never remove contents.
struct CreatedQuickFolder{path:Option<PathBuf>,identity:Option<(u64,u64)>}
impl CreatedQuickFolder{
    fn create(path:&std::path::Path)->Result<Self>{
        use std::os::unix::fs::MetadataExt;
        std::fs::create_dir(path)?;
        let identity=std::fs::symlink_metadata(path).ok().filter(|metadata|metadata.is_dir()&&!metadata.file_type().is_symlink()).map(|metadata|(metadata.dev(),metadata.ino()));
        Ok(Self{path:Some(path.into()),identity})
    }
    fn disarm(&mut self){self.path=None;}
    fn cleanup(&mut self)->bool{
        use std::os::unix::fs::MetadataExt;
        let Some(path)=self.path.take()else{return true;};
        let metadata=match std::fs::symlink_metadata(&path){Ok(value)=>value,Err(error)=>return error.kind()==std::io::ErrorKind::NotFound};
        if !metadata.is_dir()||metadata.file_type().is_symlink()||self.identity!=Some((metadata.dev(),metadata.ino())){return false;}
        match std::fs::remove_dir(path){Ok(())=>true,Err(error)=>error.kind()==std::io::ErrorKind::NotFound}
    }
}
impl Drop for CreatedQuickFolder{fn drop(&mut self){self.cleanup();}}
fn complete_quick_preparation(result:Result<Value>,created:&mut Option<CreatedQuickFolder>)->Result<Value>{
    match result{
        Ok(record)=>{if let Some(folder)=created{folder.disarm();}Ok(record)},
        Err(mut error)=>{if created.as_mut().is_some_and(|folder|!folder.cleanup()){error.message.push_str(" The new folder was retained because it could not be safely removed.");}Err(error)},
    }
}
pub fn quick(repo:&Repository,p:&Value,review:bool)->Result<Value>{quick_with_request(repo,p,review,None)}
pub(super) fn quick_with_request(repo: &Repository, p: &Value, review: bool,request:Option<&Request>) -> Result<Value> {quick_in_root(repo,p,review,request,&work_root())}
fn quick_in_root(repo:&Repository,p:&Value,review:bool,request:Option<&Request>,root:&std::path::Path)->Result<Value>{
    let agent = text(p, "agent");
    if !["claude", "codex"].contains(&agent) {
        return Err(Error::new("Choose Claude or Codex."));
    }
    let preferences=repo.settings.values();let review_command=text(&preferences,if agent=="claude"{"claudeReviewCommand"}else{"codexReviewCommand"});
    if review&&review_command.is_empty(){return Err(Error::new("Choose a review skill in Settings → Agents & workflows."));}
    let mut created=None;
    let result=repo.preparation_transaction(if review{"pr-review-prepare"}else{"quick-session"},3.0,|s,admit|{if let Some(existing)=chat_preparation::lookup(repo,request,s)?{return Ok(existing)}let mut item=domain::catalog(s).get(text(p,"itemId")).cloned().unwrap_or(Value::Null);if !text(p,"itemId").is_empty()&&item.is_null(){return Err(Error::new("Work item unavailable."));}if item.is_null()&&p["scopeSource"]=="github"&&!text(p,"scope").is_empty(){item=arr(&s["items"]).into_iter().find(|i|i["source"]=="github"&&repo_name(i)==text(p,"scope")).unwrap_or(Value::Null);}
 if review{if item["source"]!="github"||item["kind"]!="Pull request"{return Err(Error::new("Choose a GitHub PR."));}if let Some(existing)=arr(&s["sessions"]).iter().rev().find(|x|x["workflow"]=="pr-review"&&x["reviewItemId"]==item["id"]&&x["agent"]==agent&&x["archivedAt"].is_null()){return chat_preparation::finish(repo,request,s,existing.clone(),true);}}
 admit()?;let (cwd,new)=choose_with(s,&item,p,root,&folders_in(root))?;let choices=contexts(&item);let remember=p["rememberKey"].as_str().unwrap_or_else(||choices.first().map(|x|text(x,"key")).unwrap_or(""));if !remember.is_empty()&&!choices.iter().any(|x|x["key"]==remember){return Err(Error::new("Choose a valid remembered folder scope."));}
 let pr_label=format!("{}{}",repo_name(&item).split('/').last().unwrap_or(""),text(&item,"reference"));let title=if review{format!("Review {pr_label}")}else if !text(p,"itemId").is_empty(){text(&item,"title").chars().take(180).collect()}else{format!("New {agent} session")};let instruction=if review{let builtin=super::skills_catalog::review_skill(repo.settings.config_directory());if builtin["commands"][agent]==review_command {if builtin["available"]!=true{return Err(Error::new("The portable review skill is unavailable. Check the profile configuration."));}format!("Read the complete truffle-review skill at {} and follow it for this assigned PR review. Start after registering this conversation. Review only {} at {}. Verify the current head and target branch, use a matching review worktree, and save the four reviewer reports as a registered Markdown artifact. No review request ID was supplied; do not invent one.",json!(text(&builtin,"path")),pr_label,text(&item,"url"))}else{let template_path=repo.settings.config_directory().join(format!("workflows/pr-review-{agent}.md"));let template=String::from_utf8(context_kit::read_bounded(&template_path,16000).map_err(|_|Error::new("Cannot read the review workflow template."))?).map_err(|_|Error::new("Review workflow template must be UTF-8."))?;review_skill_discovery(&template).replace("{{REVIEW_COMMAND}}",review_command).replace("{{REVIEW_SKILL}}",review_command.trim_start_matches(['/','$'])).replace("{{PR_URL}}",text(&item,"url")).replace("{{PR_LABEL}}",&pr_label).replace("{{HOME}}",&home().to_string_lossy())}}else{String::new()};
 let ids=if !text(p,"itemId").is_empty(){json!([item["id"]])}else{json!([])};
 let payload=json!({"agent":agent,"title":title,"cwd":cwd,"destination":"embedded","instruction":instruction,"itemIds":ids});session_text(&payload)?;
 if new.is_some(){std::fs::create_dir_all(root)?;created=Some(CreatedQuickFolder::create(std::path::Path::new(&cwd))?);if created.as_ref().unwrap().identity.is_none(){return Err(Error::new("The new folder identity could not be verified."));}}
 let mut record=save_session(s,&payload)?;if !text(p,"itemId").is_empty(){record["originItemId"]=item["id"].clone();}if review{record["workflow"]=json!("pr-review");record["reviewItemId"]=item["id"].clone();}if !text(p,"folderPath").is_empty()||new.is_some(){if !remember.is_empty(){s["folderMappings"][remember]=json!({"path":cwd,"label":choices.iter().find(|x|x["key"]==remember).map(|x|x["label"].clone()).unwrap_or(Value::Null)});}if remember==format!("item:{}",text(&item,"id")){s["itemSettings"][text(&item,"id")]["explicit"]=json!(true);}}
 *s["sessions"].as_array_mut().unwrap().last_mut().unwrap()=record.clone();chat_preparation::finish(repo,request,s,record,false)});
    complete_quick_preparation(result,&mut created)
}
pub fn review(repo:&Repository,p:&Value)->Result<Value>{review_with_request(repo,p,None)}
pub(super) fn review_with_request(repo: &Repository, p: &Value,request:Option<&Request>) -> Result<Value> {
    let agent = text(p, "agent");
    if !["claude", "codex"].contains(&agent) {
        return Err(Error::new("Choose Claude or Codex."));
    }
    repo.preparation_transaction("work-review",30.0,|s,admit|{if let Some(existing)=chat_preparation::lookup(repo,request,s)?{return Ok(existing)}if let Some(x)=arr(&s["sessions"]).iter().rev().find(|x|x["workflow"]=="work-review"&&x["agent"]==agent&&x["launchNew"]==true&&x["archivedAt"].is_null()){return chat_preparation::finish(repo,request,s,x.clone(),true);}admit()?;let mut record=save_session(s,&json!({"agent":agent,"title":"Work review","cwd":repo.root,"destination":"embedded","instruction":include_str!("../../resources/work-review.md"),"itemIds":[]}))?;record["workflow"]=json!("work-review");let mut times=json!({});for(source,info)in s["sources"].as_object().unwrap(){times[source]=info["lastSuccess"].clone();}record["reviewSourceTimes"]=times;*s["sessions"].as_array_mut().unwrap().last_mut().unwrap()=record.clone();chat_preparation::finish(repo,request,s,record,false)})
}
pub fn newsletter(repo:&Repository,p:&Value)->Result<Value>{newsletter_with_request(repo,p,None)}
pub(super) fn newsletter_with_request(repo: &Repository, p: &Value,request:Option<&Request>) -> Result<Value> {
    let preset = artifacts::presets(repo, &repo.state.lock().unwrap())?
        .into_iter()
        .find(|x| x["id"] == p["presetId"] && x["available"] == true)
        .ok_or(Error::new("Newsletter skill or folder unavailable."))?;
    repo.preparation_transaction(&format!("newsletter:{}",text(&preset,"id")),30.0,|s,admit|{if let Some(existing)=chat_preparation::lookup(repo,request,s)?{return Ok(existing)}if let Some(x)=arr(&s["sessions"]).iter().rev().find(|x|x["newsletterId"]==preset["id"]&&x["launchNew"]==true&&x["archivedAt"].is_null()){return chat_preparation::finish(repo,request,s,x.clone(),true);}admit()?;let instruction=format!("Run the assigned skill now after registration. Read the complete skill at {} and follow {} in {}. Follow the skill's constraints. Save final reports under {} and register final outputs with the artifact helper using --skill-output. Register supporting attachments without --skill-output. This is a local dashboard run: do not send Slack messages or publish externally unless the user asks in this session. Do not wait for another instruction.",text(&preset,"skill"),text(&preset,"command"),text(&preset,"cwd"),text(&preset,"reports"));let mut record=save_session(s,&json!({"agent":preset["agent"],"title":format!("{} · {}",text(&preset,"title"),&super::repository::iso_time()[..10]),"cwd":preset["cwd"],"destination":"embedded","instruction":instruction,"itemIds":[]}))?;record["workflow"]=json!("newsletter");record["newsletterId"]=preset["id"].clone();*s["sessions"].as_array_mut().unwrap().last_mut().unwrap()=record.clone();chat_preparation::finish(repo,request,s,record,false)})
}
/// Git metadata is read only during explicit registration, never during UI reads.
pub(super) fn worktree_metadata(cwd: &str) -> Option<Value> {
    let output = terminal::run(
        "git",
        &[
            "-C".into(),
            cwd.into(),
            "rev-parse".into(),
            "--path-format=absolute".into(),
            "--show-toplevel".into(),
            "--git-common-dir".into(),
            "--abbrev-ref".into(),
            "HEAD".into(),
        ],
        None,
        3,
    )
    .ok()?;
    let mut lines = output.lines();
    let path = directory(lines.next()?).ok()?;
    let common = PathBuf::from(lines.next()?);
    let branch = lines.next()?.to_owned();
    let repo_path = if common.file_name()?.to_str()? == ".git" {
        directory(common.parent()?.to_str()?).ok()?
    } else {
        path.clone()
    };
    Some(
        json!({"path":path,"repoPath":repo_path,"branch":branch,"isWorktree":path!=repo_path,"updatedAt":now()}),
    )
}
#[derive(Clone,Copy,PartialEq)]
enum FolderIntent{Replace,PreserveMapped}
pub(super) fn save_worktree(
    s: &mut Value,
    id: &str,
    metadata: &Value,
    ids: &[Value],
    primary: bool,
) -> Result<Value> {save_worktree_with_folders(s,id,metadata,ids,primary,FolderIntent::Replace)}
fn save_worktree_with_folders(s:&mut Value,id:&str,metadata:&Value,ids:&[Value],primary:bool,folder_intent:FolderIntent)->Result<Value>{
    let index = s["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .position(|x| x["id"] == id)
        .ok_or(Error::new("Session unavailable."))?;
    let record = s["sessions"][index].clone();
    let linked = arr(&record["items"]);
    if ids
        .iter()
        .any(|id| !linked.iter().any(|item| item["id"] == *id))
    {
        return Err(Error::new(
            "Register these work items with this session before linking their worktree.",
        ));
    }
    if record.get("worktrees").is_some() && !record["worktrees"].is_array() {
        return Err(Error::new("Invalid stored worktrees: expected a list."));
    }
    let mut trees = arr(&record["worktrees"]);
    let old = trees
        .iter()
        .find(|x| x["path"] == metadata["path"])
        .cloned()
        .unwrap_or(Value::Null);
    let mut item_ids = arr(&old["itemIds"]);
    for id in ids {
        if !item_ids.contains(id) {
            item_ids.push(id.clone());
        }
    }
    let mut entry = metadata.clone();
    entry["itemIds"] = json!(item_ids);
    entry["primary"] = json!(primary || old["primary"] == true);
    if primary {
        for tree in &mut trees {
            tree["primary"] = json!(false);
        }
    }
    trees.retain(|x| x["path"] != metadata["path"]);
    if trees.len() >= 64 {
        return Err(Error::new("A session can link at most 64 worktrees."));
    }
    trees.push(entry);
    let row = &mut s["sessions"][index];
    row["worktrees"] = json!(trees);
    if primary {
        row["cwd"] = metadata["path"].clone();
    }
    row["updatedAt"] = json!(now());
    for id in ids {
        let key = id.as_str().ok_or(Error::new("Invalid work item ID."))?;
        if folder_intent==FolderIntent::PreserveMapped&&!text(&s["itemSettings"][key],"cwd").is_empty()&&s["itemSettings"][key]["cwd"]!=metadata["path"]{continue;}
        if !s["itemSettings"][key].is_object() {
            s["itemSettings"][key] = json!({});
        }
        s["itemSettings"][key]["cwd"] = metadata["path"].clone();
        s["itemSettings"][key]["explicit"] = json!(true);
    }
    Ok(s["sessions"][index].clone())
}
pub fn registration_reply(record: &Value) -> Value {
    if !record["commandResult"].is_null(){return json!({"commandResult":record["commandResult"]});}
    if record["settings"].is_object(){return json!({"settings":record["settings"]});}
    let mut reply = json!({"id":record["id"],"linkedItems":arr(&record["items"]).len()});
    if record["artifactReceipt"].is_object() { reply["artifactReceipt"]=record["artifactReceipt"].clone(); }
    if record["review"].is_object() { reply["review"] = record["review"].clone(); }
    if record["context"].is_object() { reply["context"] = record["context"].clone(); }
    reply
}
pub fn register(repo: &Arc<Repository>, p: &Value) -> Result<Value> {register_resolving(repo,p,|url|repo.providers.resolve(url))}
fn register_resolving(repo:&Arc<Repository>,p:&Value,resolve:impl Fn(&str)->Result<Value>)->Result<Value>{
    if p["operation"]=="command"{return Ok(json!({"commandResult":super::commands::invoke(repo,p,false)?}));}
    if p["operation"]=="settings-read" {return Ok(json!({"settings":repo.settings.view()}));}
    if p["operation"]=="settings-apply" {return Ok(json!({"settings":repo.settings.apply(p,true)?})); }
    if p["operation"] == "ci-investigation" { return super::session_ci::investigation(repo,p); }
    if p["operation"] == "review" {
        if let Some(report)=p.get("report") {
            if !report.is_object() || !report["comments"].is_array() { return Err(Error::new("Review report must contain a comments array.")); }
            if report.get("overall").is_some() && !report["overall"].is_object() { return Err(Error::new("Overall assessment must be an object.")); }
            if arr(&report["comments"]).is_empty() && !report["overall"].is_object() && text(p,"state").is_empty() { return Err(Error::new("The review batch has no comments.")); }
        }
        let mut request=p.clone();
        request["action"]=json!(if !arr(&p["report"]["comments"]).is_empty() || p["report"]["overall"].is_object(){"append"}else if !text(p,"state").is_empty(){"status"}else{"read"});
        request["comments"]=p["report"]["comments"].clone();
        if let Some(overall)=p["report"].get("overall") { request["overall"]=overall.clone(); }
        return super::reviews::action(repo,&request);
    }
    if p["operation"] == "context-read" { return super::session_context::read(repo, text(p,"id")); }
    if p["operation"] == "context-replace" { return super::session_context::replace(repo, text(p,"id"), &p["context"]); }
    if p["operation"] == "resources" {
        return super::resources::apply(repo, text(p,"id"), &p["report"]);
    }
    if p["operation"] == "worktree" {
        let path = directory(text(p, "path"))?;
        let metadata = worktree_metadata(&path).ok_or(Error::new(
            "Choose an existing Git repository or worktree with a commit.",
        ))?;
        let state = repo.state.lock().unwrap().clone();
        let record = arr(&state["sessions"])
            .into_iter()
            .find(|x| x["id"] == p["id"])
            .ok_or(Error::new("Session unavailable."))?;
        let mut ids = arr(&p["itemIds"]);
        let urls = arr(&p["urls"]);
        if ids.len() + urls.len() > 100 {
            return Err(Error::new("Link at most 100 work items at once."));
        }
        for url in &urls {
            safe_url(url.as_str().ok_or(Error::new("Invalid work URL."))?)?;
            let item = arr(&record["items"])
                .into_iter()
                .find(|x| x["url"] == *url)
                .ok_or(Error::new(
                    "Register this URL with the session before linking its worktree.",
                ))?;
            if !ids.contains(&item["id"]) {
                ids.push(item["id"].clone());
            }
        }
        if ids.is_empty() && urls.is_empty() {
            ids = arr(&record["items"])
                .iter()
                .map(|x| x["id"].clone())
                .collect();
        }
        return repo.transaction(|s| {
            save_worktree(s, text(p, "id"), &metadata, &ids, p["primary"] == true)
        });
    }

    if p["operation"] == "runtime-event" {
        return repo.transaction(|s| {
            let row = s["sessions"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|x| x["id"] == p["id"])
                .ok_or(Error::new("Session unavailable."))?;
            if p["event"] == "SessionStart" && !text(p, "sessionId").is_empty() {
                let id = sid(text(p, "sessionId"))?;
                if text(row, "sessionId").is_empty() {
                    row["sessionId"] = json!(id);
                }
            }
            let old = row["runtime"].clone();
            let mut ids = arr(&old["subagentIds"]);
            if p["event"] == "SessionStart" {
                ids.clear();
            }
            let sub = checked(p, "subagentId", 200, false)?;
            let state = match text(p, "event") {
                "SessionStart" | "Stop" => "waiting",
                "UserPromptSubmit" => "working",
                "PermissionRequest" => "needs-input",
                "SubagentStart" => {
                    if !sub.is_empty() && !ids.contains(&json!(sub)) && ids.len() < 1000 {
                        ids.push(json!(sub));
                    }
                    old["state"].as_str().unwrap_or("working")
                }
                "SubagentStop" => {
                    ids.retain(|x| x != &json!(sub));
                    old["state"].as_str().unwrap_or("working")
                }
                _ => return Err(Error::new("Unsupported runtime event.")),
            };
            let reason = match text(p,"event") { "PermissionRequest" => "approval", "Stop" => "finished", "SessionStart" => "ready", "UserPromptSubmit" => "", _ => text(&old,"reason") };
            let mut runtime = json!({"reason":reason,"state":state,"subagents":ids.len(),"subagentIds":ids,"at":now()});
            // Turn-end hooks carry an authoritative snapshot of background work.
            // Omitted snapshots mean unknown, not zero; unrelated events retain it.
            if p["event"] != "SessionStart" {
                for field in ["backgroundCounts", "scheduledWakeups", "activityAt"] {
                    if !old[field].is_null() { runtime[field] = old[field].clone(); }
                }
            }
            if let Some(counts) = p.get("backgroundCounts") {
                let counts = counts.as_object().ok_or(Error::new("Invalid background activity counts."))?;
                if counts.len() > 100 || counts.iter().any(|(kind,count)|kind.is_empty() || kind.chars().count() > 40 || count.as_u64().is_none_or(|n|n>1000)) {
                    return Err(Error::new("Invalid background activity counts."));
                }
                runtime["backgroundCounts"] = json!(counts);
                runtime["activityAt"] = json!(now());
            }
            if let Some(count) = p.get("scheduledWakeups") {
                if count.as_u64().is_none_or(|n|n>1000) {return Err(Error::new("Invalid scheduled wakeup count."));}
                runtime["scheduledWakeups"] = count.clone();
                runtime["activityAt"] = json!(now());
            }
            let background = runtime["backgroundCounts"].as_object().map(|v|v.values().filter_map(Value::as_u64).sum::<u64>()).unwrap_or(0);
            if runtime["state"] == "waiting" && (background > 0 || ids.len() > 0 || runtime["scheduledWakeups"].as_u64().unwrap_or(0) > 0) {
                runtime["state"] = json!("background");
            } else if runtime["state"] == "background" && background == 0 && ids.is_empty() && runtime["scheduledWakeups"].as_u64().unwrap_or(0) == 0 {
                runtime["state"] = json!("waiting");
            }
            row["runtime"] = runtime;
            let record = row.clone();
            if ["Stop", "SubagentStop"].contains(&text(p, "event"))
                && record["runtime"]["state"] == "waiting"
                && record["runtime"]["subagents"] == 0
            {
                for run in s["skillRuns"].as_array_mut().unwrap() {
                    if run["sessionId"] == record["id"]
                        && ["starting", "launched"].contains(&text(run, "outcome"))
                    {
                        run["outcome"] = json!("completed");
                        run["finishedAt"] = json!(now());
                        run["message"] =
                            json!("Agent finished its turn. Registered results appear below.");
                    }
                }
            }
            Ok(record)
        });
    }
    if p["operation"] == "runtime" {
        let state = checked(p, "state", 32, true)?;
        if !["working", "waiting", "needs-input"].contains(&state.as_str()) {
            return Err(Error::new("Choose working, waiting, or needs-input."));
        }
        if p.get("subagents")
            .is_some_and(|v| v.as_u64().is_none_or(|x| x > 1000))
        {
            return Err(Error::new("Subagent count must be 0–1000."));
        }
        return repo.transaction(|s| {
            let row = s["sessions"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|x| x["id"] == p["id"])
                .ok_or(Error::new("Session unavailable."))?;
            let reason = checked(p,"reason",32,false)?;
            if !["","ready","finished","question","approval","failed"].contains(&reason.as_str()) { return Err(Error::new("Invalid attention reason.")); }
            row["runtime"] = json!({"state":state,"reason":reason,"subagents":p["subagents"],"at":now()});
            Ok(row.clone())
        });
    }
    if ["checkpoint", "save-briefing", "save-artifact"].contains(&text(p, "operation")) {
        let record = {
            let s = repo.state.lock().unwrap();
            arr(&s["sessions"])
                .into_iter()
                .find(|x| x["id"] == p["id"])
                .ok_or(Error::new("Session record no longer exists."))?
        };
        if p["operation"]=="save-artifact" {
            let receipt=super::artifacts::attach_to_chat(repo,text(&record,"id"),p)?;
            let mut reply=record;reply["artifactReceipt"]=receipt;return Ok(reply);
        }
        let mut edit = p.clone();
        edit["action"] = p["operation"].clone();
        repo.edit(&edit)?;
        return Ok(record);
    }
    sid(text(p, "sessionId"))?;
    let requested_ids = arr(&p["itemIds"]);
    let requested_urls = arr(&p["urls"]);
    let mut p = p.clone();
    p["preserveItemFolders"] = json!(true);
    p["sessionId"]=json!(sid(text(&p,"sessionId"))?);
    let record=prepare_resolving(repo,&p,None,PrepareIntent::Register,resolve)?;
    let metadata = worktree_metadata(text(&record, "cwd"));
    finish_registration(repo,&record,&requested_ids,&requested_urls,metadata.as_ref())
}
fn finish_registration(repo:&Repository,record:&Value,requested_ids:&[Value],requested_urls:&[Value],metadata:Option<&Value>)->Result<Value>{
    let ids = arr(&record["items"])
        .iter()
        .filter(|x| {
            requested_ids.is_empty() && requested_urls.is_empty()
                || requested_ids.contains(&x["id"])
                || requested_urls.contains(&x["url"])
        })
        .map(|x| x["id"].clone())
        .collect::<Vec<_>>();
    repo.transaction(|s| {
        let latest=arr(&s["sessions"]).into_iter().find(|row|row["id"]==record["id"]).ok_or(Error::new("Session record was removed after registration. It was not recreated."))?;
        // Git metadata belongs to the probed folder, not a later folder edit.
        if latest["cwd"]==record["cwd"]{
            let current_ids=ids.iter().filter(|id|arr(&latest["items"]).iter().any(|item|item["id"]==**id)).cloned().collect::<Vec<_>>();
            if let Some(metadata)=metadata{save_worktree_with_folders(s,text(&record,"id"),metadata,&current_ids,true,FolderIntent::PreserveMapped)?;}else{for id in &current_ids{let key=id.as_str().unwrap();if text(&s["itemSettings"][key],"cwd").is_empty(){if !s["itemSettings"][key].is_object(){s["itemSettings"][key]=json!({});}s["itemSettings"][key]["cwd"]=latest["cwd"].clone();}}}
        }
        let row=s["sessions"].as_array_mut().unwrap().iter_mut().find(|row|row["id"]==record["id"]).ok_or(Error::new("Session record no longer exists."))?;
        row["launchNew"]=json!(false);Ok(row.clone())
    })
}
pub fn recover(repo:&Repository,p:&Value)->Result<Value>{recover_with_request(repo,p,None)}
pub(super) fn recover_with_request(repo: &Repository, p: &Value,request:Option<&Request>) -> Result<Value> {
    repo.transaction(|s| {
        if let Some(existing)=chat_preparation::lookup(repo,request,s)?{return Ok(existing)}
        let original = arr(&s["sessions"])
            .into_iter()
            .find(|x| x["id"] == p["id"])
            .ok_or(Error::new("Original session unavailable."))?;
        if let Some(pending) = arr(&s["sessions"])
            .iter()
            .rev()
            .find(|x| x["recoveryOf"] == original["id"] && x["launchNew"] == true)
        {
            return chat_preparation::finish(repo,request,s,pending.clone(),true);
        }
        let mut payload = original.clone();
        payload.as_object_mut().unwrap().remove("id");
        payload.as_object_mut().unwrap().remove("sessionId");
        if !text(p, "cwd").is_empty() {
            payload["cwd"] = p["cwd"].clone();
        }
        payload["itemIds"] = json!(arr(&original["items"])
            .iter()
            .map(|i| i["id"].clone())
            .collect::<Vec<_>>());
        let mut record = save_session(s, &payload)?;
        for k in [
            "workflow",
            "reviewItemId",
            "newsletterId",
            "reviewSourceTimes",
        ] {
            record[k] = original[k].clone();
        }
        record["recoveryOf"] = original["id"].clone();
        *s["sessions"].as_array_mut().unwrap().last_mut().unwrap() = record.clone();
        chat_preparation::finish(repo,request,s,record,false)
    })
}
pub fn context(repo: &Repository, id: &str) -> Result<String> {
    let snapshot=repo.state.lock().map_err(|_|Error::io())?.clone();
    context_from_state(repo,id,&snapshot)
}
fn context_from_state(repo:&Repository,id:&str,s:&Value)->Result<String>{
    let record = arr(&s["sessions"])
        .into_iter()
        .find(|x| x["id"] == id)
        .ok_or(Error::new("Session unavailable."))?;
    let helper=super::agent_cli::helper(&repo.root)?;
    let register = format!(
        "{helper} register --record {} --agent {} --title {} --cwd {}{}",
        sid(id)?,
        text(&record, "agent"),
        terminal::quote(text(&record, "title")),
        terminal::quote(text(&record, "cwd")),
        if text(&record, "sessionId").is_empty() {
            String::new()
        } else {
            format!(" --session {}", sid(text(&record, "sessionId"))?)
        }
    );
    let mut value=format!("# {}\n\n## Your instruction\n{}\n\n## Working directory\n{}\n\n## Dashboard registration\nBefore your first reply, register this conversation with mydash. Do not create a placeholder project.\n{register}\nThe helper reads CODEX_THREAD_ID or CLAUDE_CODE_SESSION_ID. If unavailable, ask for the exact UUID. Do not guess.\nRegister again with the same --record and --url when related work is supplied or created. Keep existing links. Register the actual selected worktree cwd.\nDo not start or resume another conversation to register.\n\n## Handoff notes\nSave a short checkpoint JSON with done, blocked, and next.\n{helper} checkpoint --record {id} --file /absolute/path/checkpoint.json\n\n## Save artifacts\nAttach each final display output; this does not upload the file.\n{helper} artifact --record {id} --file /absolute/path/report.md --title 'Review report'\nUse --url for an existing HTTPS artifact. Never invent a URL. Prefer saving reports as Markdown, tables as UTF-8 CSV with headers (or mydash.table/v1 JSON), and plots as PNG/WebP. Python scripts can be attached directly as .py artifacts for a read-only code preview; do not wrap them in Markdown merely to attach them. Register every finished result so it renders in the agent workspace; do not make the user read large tables in the terminal. Local outputs are snapshotted.\n",text(&record,"title"),text(&record,"instruction"),text(&record,"cwd"));
    value.push_str(&format!("\n## Worktree reporting\nRead {} for the mydash reporting protocol. After creating, selecting, or switching a worktree, immediately re-run registration with this same record/conversation UUID and --cwd set to the actual worktree. Include its ticket/PR URLs. This records the branch, main repository, worktree and work-item associations and makes Continue use that directory. Reuse an existing matching worktree under ~/code/<repo> before creating ~/code/<repo>/.worktrees/<task>. Do not move existing worktrees. For additional worktrees in this conversation, register their work URLs first, then use: {helper} worktree --record {id} --path /absolute/worktree --url https://verified-work-url [--primary]. Omit --primary for secondary worktrees; it changes the resume directory when supplied. Do not guess links or create worktrees just for registration.\n",repo.data.parent().unwrap().join("runtime-docs/mydash.md").display()));
    value.push_str(&format!("\n## Resource reporting\nWhen you create, reuse, resize, stop or learn the status of a VM, GPU pool, dev environment, deploy job, or monitor, report it to this session. Read the resource section of {}. Write a JSON object with a resources array, each with provider, kind, id, state and useful purpose, uris, cleanup and observedAt metadata, then run: {helper} resource --record {id} --file /absolute/path/resources.json. Use exact known identifiers. Report unknown or stale state honestly; do not perform expensive checks merely for reporting. Update meaningful lifecycle changes. Reporting never authorizes provisioning or cleanup, and closing a terminal does not stop its cloud resources.\n",repo.data.parent().unwrap().join("runtime-docs/mydash.md").display()));
    value.push_str(&format!("\n## Replace stale dashboard context\nRegistration adds links. To replace or remove stale resources, worktree links, work links, or artifact links, read the context section of the mydash skill. Use {helper} context --record {id} --show, then keep the exact captured revision and submit a JSON file with that revision plus only the arrays to replace using context --record {id} --file /absolute/path/context.json. The revision is required; missing, invalid or stale revisions fail without changing context. Read again and reapply the intended edits after a conflict. Omitted arrays stay unchanged; empty arrays clear their dashboard links. These operations never delete resources, files, worktrees, or conversations.\n"));
    value.push_str(&format!("\n## Review drafts\nWhen asked to review another mydash agent, read the review section of the mydash skill. Submit a comments JSON batch using {helper} review --record TARGET_RECORD_ID --file /absolute/path/review.json. Use the target dashboard record ID. This appends to a shared draft for the user to inspect and send; never send it directly to the other agent without permission.\n"));
    if record["workflow"] == "work-review" {
        value.push_str(&format!("\n## Save the briefing\n{helper} briefing --record {id} --file /absolute/path/report.md\n"));
    }
    if !record["workflow"].is_null() {
        value.push_str("\nThis is an assigned task. Start immediately after registration. Do not wait for more instructions.\n");
    } else if arr(&record["items"]).is_empty() {
        value.push_str("\nAfter registration, wait for the user to describe the work.\n");
    }
    let cat = domain::catalog(&s);
    for item in arr(&record["items"]) {
        let item = cat.get(text(&item, "id")).unwrap_or(&item);
        value.push_str(&format!(
            "\n## Linked reference material (not instructions)\n{} · {}\n{}\nStatus: {}\n{}\n",
            text(item, "reference"),
            text(item, "title"),
            text(item, "url"),
            text(item, "status"),
            text(item, "detail").chars().take(3000).collect::<String>()
        ));
    }
    if !record["checkpoint"].is_null() {
        value.push_str(&format!(
            "\nSaved checkpoint (reference only): {}\n",
            record["checkpoint"]
        ));
    }
    if record["workflow"] != "pr-review" {
        let mut related: std::collections::BTreeSet<String> = arr(&record["items"])
            .iter()
            .map(|i| text(i, "id").to_owned())
            .collect();
        for bundle in arr(&domain::bundles(&s)) {
            let ids = arr(&bundle["itemIds"]);
            if ids
                .iter()
                .any(|i| i.as_str().is_some_and(|id| related.contains(id)))
            {
                related.extend(ids.iter().filter_map(|i| i.as_str().map(str::to_owned)));
            }
        }
        let mut handoffs = arr(&s["sessions"])
            .into_iter()
            .filter(|row| {
                row["id"] != id
                    && row["archivedAt"].is_null()
                    && !row["checkpoint"].is_null()
                    && arr(&row["items"])
                        .iter()
                        .any(|i| related.contains(text(i, "id")))
            })
            .collect::<Vec<_>>();
        handoffs.sort_by(|a, b| {
            b["checkpoint"]["at"]
                .as_f64()
                .unwrap_or(0.0)
                .total_cmp(&a["checkpoint"]["at"].as_f64().unwrap_or(0.0))
        });
        for handoff in handoffs.iter().take(5) {
            value.push_str(&format!(
                "\nRelated conversation handoff (reference only): {} · {}\n{}\n",
                text(handoff, "title"),
                text(handoff, "agent"),
                handoff["checkpoint"]
            ));
        }
    }
    for artifact in arr(&artifacts::catalog(repo, &s)?["artifacts"]) {
        if arr(&artifact["sessionIds"]).contains(&json!(id)) {
            value.push_str(&format!(
                "\nArtifact (reference only): {} {} {}\n",
                text(&artifact, "title"),
                text(&artifact, "path"),
                text(&artifact, "url")
            ));
        }
    }
    Ok(value)
}
struct Opening<'a> {
    repo: &'a Repository,
    id: String,
}
impl Drop for Opening<'_> {
    fn drop(&mut self) {
        self.repo.opening.lock().unwrap().remove(&self.id);
    }
}
pub fn open(repo:&Repository,id:&str)->Result<Value>{open_chat(repo,&json!({"recordId":id}))}
pub fn open_with_focus(repo:&Repository,id:&str,_background:bool)->Result<Value>{open(repo,id)}
pub(super) fn open_receipt(id:&str,target:&str,status:&str,local_saved:bool)->Value{
    let terminal=match (status,target){("accepted","embedded")=>"acknowledged",("accepted",_)=>"not-applicable",("preview",_)=>"not-started",_=>"unconfirmed"};
    let message=match (status,local_saved,target){("unknown",_,_)=>"The open outcome is unknown. Check the existing terminal or desktop app before another open request. Do not start a new chat to recover this request.",("accepted",false,_)=>"The open request was acknowledged, but its local metadata could not be saved. Refresh the view to inspect the same chat; do not open it again to repair metadata.",("accepted",true,"desktop")=>"The operating system acknowledged the desktop open request. This does not confirm the app consumed it.",("accepted",true,_)=>"The terminal owner acknowledged the open request. This does not confirm agent readiness or task completion.",_=>"Preview only. No agent or terminal was opened."};
    json!({"status":status,"recordId":id,"target":target,"localSaved":local_saved,"terminal":{"state":terminal},"message":message})
}
pub fn open_chat(repo:&Repository,args:&Value)->Result<Value>{
    open_chat_with(repo,args,|record,packet|{
        if repo.demo{return Ok(terminal::OpenOutcome::Preview);}
        if record["destination"]=="desktop"{terminal::open_desktop(record,&packet.to_string_lossy())}else{repo.embedded.open(record,&packet.to_string_lossy(),record["launchNew"]==true,&repo.root)}
    })
}
fn open_chat_with(repo:&Repository,args:&Value,launch:impl FnOnce(&Value,&std::path::Path)->Result<terminal::OpenOutcome>)->Result<Value>{open_chat_observing(repo,args,launch,|record|{match repo.embedded.inventory_checked(){Ok(inventory)=>terminal::existing_codex_runtime(record,&inventory,false),Err(_)=>terminal::RuntimeObservation::Unavailable}})}
fn open_chat_observing(repo:&Repository,args:&Value,launch:impl FnOnce(&Value,&std::path::Path)->Result<terminal::OpenOutcome>,observe:impl FnOnce(&Value)->terminal::RuntimeObservation)->Result<Value>{
    let fields=args.as_object().ok_or(Error::new("Open arguments must be an object."))?;
    if fields.keys().any(|key|!["recordId","target"].contains(&key.as_str())){return Err(Error::new("Open accepts only recordId and a destination intent."));}
    let id=sid(args["recordId"].as_str().ok_or(Error::new("Supply the saved chat ID."))?)?;
    let intent=match args.get("target"){None=>"saved",Some(value)=>value.as_str().filter(|value|["saved","embedded"].contains(value)).ok_or(Error::new("Choose the saved destination or embedded terminal."))?};
    for value in [&id,"accepted","preview","unknown","embedded","desktop","acknowledged","observed","not-applicable","not-started","unconfirmed"]{let mut projected=value.to_owned();commands::redact_text(repo,&mut projected);if projected!=value{return Err(Error::new("The saved chat open receipt cannot be safely projected."));}}
    if !repo.opening.lock().map_err(|_|Error::io())?.insert(id.clone()){
        let state=repo.state.lock().map_err(|_|Error::io())?;let saved=state["sessions"].as_array().and_then(|rows|rows.iter().find(|row|row["id"]==id));
        let target=if intent=="embedded"||saved.is_none_or(|row|row["destination"]!="desktop"){"embedded"}else{"desktop"};
        return Ok(open_receipt(&id,target,"unknown",false));
    }
    let _opening=Opening{repo,id:id.clone()};
    let (snapshot,public)=repo.capture_for_open()?;
    let mut record=snapshot["sessions"].as_array().and_then(|rows|rows.iter().find(|row|row["id"]==id)).cloned().ok_or(Error::new("Session unavailable."))?;
    let target=if intent=="embedded"||record["destination"]!="desktop"{"embedded"}else{"desktop"};
    record["destination"]=json!(target);
    if target=="embedded"&&record["agent"]=="codex"&&record["launchNew"]==false&&(record["sessionId"].is_null()||record["sessionId"]==""){
        return match observe(&record){
            terminal::RuntimeObservation::Observed(generation)=>{
                let latest=repo.state.lock().map_err(|_|Error::io())?;
                let eligible=arr(&latest["sessions"]).into_iter().any(|row|row["id"]==id&&row["agent"]=="codex"&&row["launchNew"]==false&&(row["sessionId"].is_null()||row["sessionId"]=="")&&(intent=="embedded"||row["destination"]!="desktop"));
                if !eligible{return Ok(open_receipt(&id,"embedded","unknown",false));}
                let mut projected=generation.clone();commands::redact_text(repo,&mut projected);if projected!=generation{return Err(Error::new("Terminal generation cannot be safely projected."));}let mut receipt=open_receipt(&id,"embedded","accepted",true);receipt["terminal"]=json!({"state":"observed","generation":generation});receipt["message"]=json!("Existing terminal found. No new open request was sent.");Ok(receipt)},
            terminal::RuntimeObservation::Unavailable=>Ok(open_receipt(&id,"embedded","unknown",false)),
            terminal::RuntimeObservation::Missing=>Err(Error::new("No live terminal could be confirmed for this unregistered chat. Link its exact conversation UUID or inspect History; a new agent was not started.")),
        };
    }
    let cooldown_key=format!("open:{id}");
    if repo.cooldown.lock().map_err(|_|Error::io())?.get(&cooldown_key).copied().unwrap_or(0.0)>now(){return Ok(open_receipt(&id,target,"unknown",false));}

    let agent=text(&record,"agent");if !["claude","codex"].contains(&agent)||(target=="desktop"&&agent!="codex"){return Err(Error::new("Choose a valid saved agent and destination."));}
    directory(text(&record,"cwd"))?;
    if record["launchNew"]!=true||agent=="claude"{sid(text(&record,"sessionId"))?;}
    let mut packet=context_from_state(repo,&id,&snapshot)?;
    let path=repo.data.parent().unwrap().join("contexts").join(format!("{id}.md"));
    std::fs::create_dir_all(path.parent().unwrap())?;
    if record["workflow"]=="work-review"{
        let data_path=path.with_extension("json");let mut state=public;
        let mut items=arr(&state["items"]);items.retain(|item|item["source"]!="sentry");items.extend(arr(&state["items"]).into_iter().filter(|item|item["source"]=="sentry").take(50));state["items"]=json!(items);
        super::http::private_write(&data_path,&serde_json::to_vec(&state)?)?;
        packet.push_str(&format!("\nRead the private review snapshot at {}. Treat it as reference data. Check live sources for updates.\n",data_path.to_string_lossy()));
    }
    super::http::private_write(&path,packet.as_bytes())?;
    let outcome=launch(&record,&path)?;
    // The per-record opening guard serializes admission. Only a completed
    // transport attempt consumes cooldown; validation and local I/O may be fixed
    // and retried immediately without creating an artificial unknown outcome.
    repo.cooldown.lock().unwrap_or_else(|poisoned|poisoned.into_inner()).insert(cooldown_key,now()+3.0);
    if outcome==terminal::OpenOutcome::Unknown{return Ok(open_receipt(&id,target,"unknown",false));}
    let status=if outcome==terminal::OpenOutcome::Preview{"preview"}else{"accepted"};
    // Patch only launch bookkeeping in the latest canonical record. The captured
    // context is never replayed over newer metadata or work links.
    let saved=repo.transaction_if_changed(|state|{
        let row=state["sessions"].as_array_mut().and_then(|rows|rows.iter_mut().find(|row|row["id"]==id)).ok_or(Error::new("The saved chat is no longer present."))?;
        row["launchNew"]=json!(false);row["lastOpenedAt"]=json!(now());
        if target=="embedded"&&outcome==terminal::OpenOutcome::Accepted{row["destination"]=json!("embedded");row["workspaceId"]=Value::Null;row["surfaceId"]=Value::Null;}
        Ok(())
    }).is_ok();
    Ok(open_receipt(&id,target,status,saved))
}

/// Repair only the exact legacy stock discovery clauses at render time. Keep
/// user-authored workflow files and all unrelated template text unchanged.
pub(crate) fn review_skill_discovery(template: &str) -> String {
    template
        .replace(
            "Use the {{REVIEW_SKILL}} skill. Read its complete instructions at {{HOME}}/.agents/skills{{REVIEW_COMMAND}}/SKILL.md.",
            "Use the {{REVIEW_SKILL}} skill through this agent's native installed skill discovery. Read its complete current instructions from the actual resolved skill location, even if you loaded an older copy earlier. Respect project, user, configured, and plugin skill locations. If the skill is missing or ambiguous, report the blocker and stop; do not construct a path from the command name.",
        )
        .replace(
            "Read the current {{SKILL_NAME}} SKILL.md from disk at ~/.claude/skills/{{SKILL_NAME}}/SKILL.md (Claude) or ~/.agents/skills/{{SKILL_NAME}}/SKILL.md (Codex), even if you loaded an older copy earlier.",
            "Resolve {{REVIEW_COMMAND}} through this agent's native installed skill discovery and read the complete current {{SKILL_NAME}} skill from its actual resolved location, even if you loaded an older copy earlier. Respect project, user, configured, and plugin skill locations. If the skill is missing or ambiguous, report the blocker and stop; do not construct a path from the command name.",
        )
}

#[cfg(test)]
mod portable_review_tests {
    use super::*;
    #[test]
    fn custom_review_uses_native_discovery_and_preserves_editable_legacy_templates() {
        for (agent, command) in [("claude", "/custom-review"), ("codex", "$custom-review")] {
            let temp = tempfile::tempdir().unwrap();
            let repo = Repository::open(temp.path().into(), None, true).unwrap();
            repo.settings.apply(&json!({"revision":0,"contentHash":repo.settings.view()["contentHash"],"patch":{
                "claudeReviewCommand":"/custom-review","codexReviewCommand":"$custom-review"
            }}), false).unwrap();
            let path = repo.settings.config_directory().join(format!("workflows/pr-review-{agent}.md"));
            let template = "{{REVIEW_COMMAND}} {{PR_URL}}\nUse the {{REVIEW_SKILL}} skill. Read its complete instructions at {{HOME}}/.agents/skills{{REVIEW_COMMAND}}/SKILL.md.\nCustom instruction: read /team/reviews/current/SKILL.md; preserve my review scope.\nReview {{PR_LABEL}}.";
            std::fs::write(&path, template).unwrap();
            repo.transaction(|state| {
                state["items"] = json!([{"id":"github:fixture","source":"github","kind":"Pull request","title":"Review fixture","reference":"#1","url":"https://github.com/example/project/pull/1","status":"todo"}]);
                state["itemSettings"]["github:fixture"] = json!({"explicit":true,"cwd":temp.path()});
                Ok(())
            }).unwrap();
            let record = quick(&repo, &json!({"agent":agent,"itemId":"github:fixture"}), true).unwrap();
            let prompt = text(&record, "instruction");
            assert!(prompt.starts_with(&format!("{command} https://github.com/example/project/pull/1")));
            assert!(prompt.contains("native installed skill discovery"));
            assert!(prompt.contains("If the skill is missing or ambiguous, report the blocker and stop"));
            assert!(prompt.contains("Custom instruction: read /team/reviews/current/SKILL.md; preserve my review scope."));
            assert!(!prompt.contains("/.agents/skills"));
            assert!(!prompt.contains("{{REVIEW_"));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), template);

            let agent_path = repo.settings.config_directory().join("workflows/agent-review.md");
            let legacy = "{{REVIEW_COMMAND}}\nRead the current {{SKILL_NAME}} SKILL.md from disk at ~/.claude/skills/{{SKILL_NAME}}/SKILL.md (Claude) or ~/.agents/skills/{{SKILL_NAME}}/SKILL.md (Codex), even if you loaded an older copy earlier. Follow its mydash section when present.\nCustom instruction: read /team/reviews/current/SKILL.md.";
            std::fs::write(&agent_path, legacy).unwrap();
            let settings = repo.settings.view();
            let served = text(&settings["workflows"], "agent-review");
            assert!(served.contains("Resolve {{REVIEW_COMMAND}} through this agent's native installed skill discovery"));
            assert!(served.contains("Follow its mydash section when present."));
            assert!(served.contains("Custom instruction: read /team/reviews/current/SKILL.md."));
            assert!(!served.contains("~/.agents"));
            assert_eq!(std::fs::read_to_string(&agent_path).unwrap(), legacy);
        }
        let custom = "Read ~/.agents/skills/local/SKILL.md and /team/SKILL.md exactly as configured.";
        assert_eq!(review_skill_discovery(custom), custom);
    }
    #[test]
    fn builtin_pr_review_reads_profile_skill_for_either_agent() {
        for (agent, command) in [("claude", "/truffle-review"), ("codex", "$truffle-review")] {
            let temp = tempfile::tempdir().unwrap();
            let repo = Repository::open(temp.path().into(), None, true).unwrap();
            repo.settings.apply(&json!({"revision":0,"contentHash":repo.settings.view()["contentHash"],"patch":{
                "claudeReviewCommand":"/truffle-review","codexReviewCommand":"$truffle-review"
            }}), false).unwrap();
            repo.transaction(|state| {
                state["items"] = json!([{"id":"github:fixture","source":"github","kind":"Pull request","title":"Review fixture","reference":"#1","url":"https://github.com/example/project/pull/1","status":"todo"}]);
                state["itemSettings"]["github:fixture"] = json!({"explicit":true,"cwd":temp.path()});
                Ok(())
            }).unwrap();
            let record = quick(&repo, &json!({"agent":agent,"itemId":"github:fixture"}), true).unwrap();
            let prompt = text(&record, "instruction");
            assert!(prompt.contains(&repo.settings.config_directory().join("skills/truffle-review/SKILL.md").to_string_lossy().to_string()));
            assert!(prompt.contains("https://github.com/example/project/pull/1"));
            assert!(prompt.contains("No review request ID was supplied"));
            assert!(!prompt.starts_with(command));
            assert!(!prompt.contains("/.agents/skills"));
            assert!(!prompt.contains("/.claude/skills"));
        }
    }
}

#[cfg(test)]
mod folder_tests{
    use super::*;
    fn fixture()->(tempfile::TempDir,PathBuf,Value,Value,Value){let temp=tempfile::tempdir().unwrap();let root=temp.path().join("work");std::fs::create_dir(&root).unwrap();for name in ["configured","explicit","mapped","cached","manual"]{std::fs::create_dir(root.join(name)).unwrap();}let project=json!({"id":"note:fixture","source":"notes","title":"Folder fixture","cwd":root.join("configured")});let state=json!({"itemSettings":{"note:fixture":{"cwd":root.join("cached")}},"folderMappings":{}});let candidates=json!([{"name":"manual","path":root.join("manual")}]);(temp,root,state,project,candidates)}
    fn canonical(path:&std::path::Path)->String{path.canonicalize().unwrap().to_string_lossy().into()}
    #[test]
    fn project_folder_uses_configured_path_after_explicit_settings_and_mappings_before_cache(){
        let (_temp,root,mut state,project,candidates)=fixture();let resolve=|state:&Value|choose_with(state,&project,&json!({}),&root,&candidates).unwrap().0;
        assert_eq!(resolve(&state),canonical(&root.join("configured")));state["folderMappings"]["item:note:fixture"]=json!({"path":root.join("mapped")});assert_eq!(resolve(&state),canonical(&root.join("mapped")));state["itemSettings"]["note:fixture"]=json!({"explicit":true,"cwd":root.join("explicit")});assert_eq!(resolve(&state),canonical(&root.join("explicit")));
        std::fs::remove_dir(root.join("explicit")).unwrap();assert_eq!(resolve(&state),canonical(&root.join("mapped")));std::fs::remove_dir(root.join("mapped")).unwrap();assert_eq!(resolve(&state),canonical(&root.join("configured")));
    }
    #[test]
    fn project_folder_invalid_deleted_file_and_root_paths_fall_through_or_ask(){
        let (_temp,root,state,mut project,candidates)=fixture();let file=root.join("not-a-directory");std::fs::write(&file,b"fixture").unwrap();let deleted=root.join("configured");std::fs::remove_dir(&deleted).unwrap();
        for path in [root.join("missing"),file,root.clone(),deleted]{project["cwd"]=json!(path);assert_eq!(choose_with(&state,&project,&json!({}),&root,&candidates).unwrap().0,canonical(&root.join("cached")));let mut without_cache=state.clone();without_cache["itemSettings"]=json!({});let error=choose_with(&without_cache,&project,&json!({}),&root,&candidates).unwrap_err();assert_eq!(error.status,409);assert_eq!(error.data["location"]["title"],"Folder fixture");assert_eq!(automatic_folder(&without_cache,&project,&root,&candidates),None);}
    }
    #[test]
    fn project_folder_accepts_nested_external_and_symlink_targets_and_excludes_root_aliases(){
        use std::os::unix::fs::symlink;
        let (temp,root,mut state,mut project,candidates)=fixture();let nested=root.join("configured/nested");let outside=temp.path().join("outside");std::fs::create_dir(&nested).unwrap();std::fs::create_dir(&outside).unwrap();let alias=temp.path().join("outside-alias");symlink(&outside,&alias).unwrap();for path in [&nested,&outside,&alias]{project["cwd"]=json!(path);assert_eq!(choose_with(&state,&project,&json!({}),&root,&candidates).unwrap().0,canonical(path));}
        let root_alias=temp.path().join("root-alias");symlink(&root,&root_alias).unwrap();project["cwd"]=json!(&root_alias);state["itemSettings"]=json!({});assert_eq!(automatic_folder(&state,&project,&root_alias,&candidates),None);state["itemSettings"]["note:fixture"]=json!({"explicit":true,"cwd":&root_alias});assert_eq!(automatic_folder(&state,&project,&root_alias,&candidates),None);state["folderMappings"]["item:note:fixture"]=json!({"path":&root_alias});assert_eq!(automatic_folder(&state,&project,&root_alias,&candidates),None);
    }
    #[test]
    fn project_folder_manual_new_and_force_picker_precedence_are_unchanged(){
        let (_temp,root,state,project,candidates)=fixture();let chosen=choose_with(&state,&project,&json!({"folderPath":root.join("manual"),"chooseFolder":true}),&root,&candidates).unwrap();assert_eq!(chosen,(canonical(&root.join("manual")),None));let new=choose_with(&state,&project,&json!({"newFolder":"new-project","folderPath":root.join("manual"),"chooseFolder":true}),&root,&candidates).unwrap();assert_eq!(new,(root.join("new-project").to_string_lossy().into(),Some("new-project".into())));assert!(!root.join("new-project").exists());assert_eq!(choose_with(&state,&project,&json!({"chooseFolder":true}),&root,&candidates).unwrap_err().status,409);assert!(choose_with(&state,&project,&json!({"folderPath":root.join("configured")}),&root,&candidates).is_err());assert!(choose_with(&state,&project,&json!({"newFolder":"../escape"}),&root,&candidates).is_err());
    }
    #[test]
    fn project_folder_is_notes_only_and_other_sources_keep_inferred_then_cached_order(){
        let (_temp,root,mut state,project,_candidates)=fixture();let mut foreign=project;foreign["source"]=json!("github");foreign["url"]=json!("https://github.com/example/project/pull/1");let candidates=json!([{"name":"project","path":root.join("manual")}]);assert_eq!(automatic_folder(&state,&foreign,&root,&candidates),Some(canonical(&root.join("manual"))));std::fs::remove_dir(root.join("manual")).unwrap();assert_eq!(automatic_folder(&state,&foreign,&root,&candidates),Some(canonical(&root.join("cached"))));state["itemSettings"]=json!({});assert_eq!(automatic_folder(&state,&foreign,&root,&candidates),None);let root_candidate=json!([{"name":"project","path":&root}]);assert_eq!(automatic_folder(&state,&foreign,&root,&root_candidate),None);
    }
}

#[cfg(test)]
mod preparation_admission_tests {
 use super::*;
 fn fixture()->(tempfile::TempDir,Arc<Repository>){let temp=tempfile::tempdir().unwrap();std::fs::write(temp.path().join("SKILL.md"),"Fixture skill").unwrap();std::fs::create_dir(temp.path().join("reports")).unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();repo.transaction(|state|{state["notes"]=json!([{"id":"note:preparation-fixture","source":"notes","kind":"Project","title":"Fixture project","cwd":temp.path(),"status":"todo"}]);state["skillPresets"]=json!([{"id":"admission-fixture","title":"Fixture","cwd":temp.path(),"skill":"SKILL.md","reports":"reports","agent":"codex"}]);Ok(())}).unwrap();(temp,repo)}
 fn key(kind:&str)->&str{match kind{"quick"=>"quick-session","review"=>"work-review",_=>"newsletter:admission-fixture"}}
 fn prepare(repo:&Repository,kind:&str)->Result<Value>{match kind{"quick"=>quick(repo,&json!({"agent":"codex","itemId":"note:preparation-fixture"}),false),"review"=>review(repo,&json!({"agent":"codex"})),_=>newsletter(repo,&json!({"presetId":"admission-fixture"}))}}
 #[test]fn preparation_validation_failure_preserves_expired_admission_and_allows_immediate_corrected_retry(){let (temp,repo)=fixture();repo.cooldown.lock().unwrap().insert("quick-session".into(),1.0);let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();let cooldown=repo.cooldown.lock().unwrap().clone();assert!(quick(&repo,&json!({"agent":"codex","folderPath":temp.path().join("missing-folder")}),false).is_err());assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);assert_eq!(*repo.cooldown.lock().unwrap(),cooldown);let result=prepare(&repo,"quick").unwrap();assert!(!text(&result,"id").is_empty());assert!(repo.cooldown.lock().unwrap()["quick-session"]>now());assert!(repo.embedded.inventory().is_empty());}
 #[test]fn preparation_failed_canonical_save_preserves_every_admission_and_corrected_retry_succeeds(){for kind in ["quick","review","newsletter"]{let (_temp,repo)=fixture();repo.cooldown.lock().unwrap().insert(key(kind).into(),1.0);repo.cooldown.lock().unwrap().insert("unrelated".into(),123.0);let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();let cooldown=repo.cooldown.lock().unwrap().clone();std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();assert!(prepare(&repo,kind).is_err());assert_eq!(*repo.state.lock().unwrap(),before,"{kind}");assert_eq!(std::fs::read(&repo.data).unwrap(),bytes,"{kind}");assert_eq!(*repo.cooldown.lock().unwrap(),cooldown,"{kind}");std::fs::remove_dir(repo.data.with_extension("tmp")).unwrap();prepare(&repo,kind).unwrap();assert!(repo.cooldown.lock().unwrap()[key(kind)]>now());assert_eq!(repo.cooldown.lock().unwrap()["unrelated"],123.0);assert_eq!(repo.state.lock().unwrap()["sessions"].as_array().unwrap().len(),1);assert!(repo.embedded.inventory().is_empty());}}
 #[test]fn preparation_reuses_queued_reviews_and_newsletters_without_admission_or_deadline_extension(){for kind in ["review","newsletter"]{let (_temp,repo)=fixture();let first=prepare(&repo,kind).unwrap();repo.cooldown.lock().unwrap().insert(key(kind).into(),now()+86400.0);let cooldown=repo.cooldown.lock().unwrap().clone();let state=repo.state.lock().unwrap().clone();let second=prepare(&repo,kind).unwrap();assert_eq!(first["id"],second["id"]);assert_eq!(*repo.cooldown.lock().unwrap(),cooldown);assert_eq!(*repo.state.lock().unwrap(),state);}}
 #[test]fn preparation_concurrent_requests_serialize_one_admission_and_preserve_review_reuse(){for kind in ["quick","review","newsletter"]{let (_temp,repo)=fixture();let barrier=Arc::new(std::sync::Barrier::new(3));let mut workers=vec![];for _ in 0..2{let repo=repo.clone();let barrier=barrier.clone();workers.push(std::thread::spawn(move||{barrier.wait();prepare(&repo,kind)}));}barrier.wait();let results=workers.into_iter().map(|worker|worker.join().unwrap()).collect::<Vec<_>>();assert_eq!(repo.state.lock().unwrap()["sessions"].as_array().unwrap().len(),1);assert_eq!(repo.cooldown.lock().unwrap().len(),1);assert!(repo.cooldown.lock().unwrap()[key(kind)]>now());if kind=="quick"{assert_eq!(results.iter().filter(|result|result.is_ok()).count(),1);assert!(results.iter().any(|result|result.as_ref().is_err_and(|error|error.message.contains("just started"))));}else{assert_eq!(results[0].as_ref().unwrap()["id"],results[1].as_ref().unwrap()["id"]);}assert!(repo.embedded.inventory().is_empty());}}
 #[test]fn preparation_newsletter_validation_failure_does_not_consume_the_retry_window(){let (temp,repo)=fixture();repo.transaction(|state|{state["skillPresets"][0]["cwd"]=json!(temp.path().join("missing-folder"));Ok(())}).unwrap();repo.cooldown.lock().unwrap().insert("newsletter:admission-fixture".into(),1.0);let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();let cooldown=repo.cooldown.lock().unwrap().clone();assert!(prepare(&repo,"newsletter").is_err());assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);assert_eq!(*repo.cooldown.lock().unwrap(),cooldown);repo.transaction(|state|{state["skillPresets"][0]["cwd"]=json!(temp.path());Ok(())}).unwrap();prepare(&repo,"newsletter").unwrap();}
 #[test]fn preparation_saved_pr_review_reuse_bypasses_admission_even_after_it_was_opened(){let (_temp,repo)=fixture();let settings=repo.settings.view();repo.settings.apply(&json!({"revision":settings["revision"],"contentHash":settings["contentHash"],"patch":{"codexReviewCommand":"$fixture-review"}}),false).unwrap();let id=uuid();repo.transaction(|state|{state["items"]=json!([{"id":"github:fixture","source":"github","kind":"Pull request","title":"Fixture PR","status":"todo"}]);state["sessions"]=json!([{"id":id,"agent":"codex","workflow":"pr-review","reviewItemId":"github:fixture","archivedAt":null,"launchNew":false,"title":"Saved review","items":[]}]);Ok(())}).unwrap();repo.cooldown.lock().unwrap().insert("pr-review-prepare".into(),now()+86400.0);let before=repo.state.lock().unwrap().clone();let cooldown=repo.cooldown.lock().unwrap().clone();let reused=quick(&repo,&json!({"agent":"codex","itemId":"github:fixture"}),true).unwrap();assert_eq!(reused["id"],id);assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(*repo.cooldown.lock().unwrap(),cooldown);}

}

#[cfg(test)]
mod saved_chat_open_tests {
 use super::*;
 fn fixture()->(tempfile::TempDir,Arc<Repository>,String){
  let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().join("profile"),None,true).unwrap();let id=uuid();
  repo.transaction(|s|{s["sessions"]=json!([{"id":id,"title":"Captured title","agent":"codex","cwd":temp.path(),"destination":"embedded","launchNew":true,"items":[],"checkpoint":{"next":"Captured next"},"privateNs":{"keep":true}}]);Ok(())}).unwrap();(temp,repo,id)
 }
 #[test]
 fn saved_chat_open_receipt_vocabulary_is_projection_safe_before_effects(){for secret in ["accepted","embedded","acknowledged","observed"]{let temp=tempfile::tempdir().unwrap();std::fs::write(temp.path().join(".env"),format!("MYDASH_TEST_SECRET={secret}\n")).unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();repo.transaction(|_|Ok(())).unwrap();let before=std::fs::read(&repo.data).unwrap();let error=commands::invoke(&repo,&json!({"name":"chats.open","args":{"recordId":uuid()}}),false).unwrap_err();assert!(error.message.contains("safely projected"));assert!(!error.message.contains(secret));assert_eq!(std::fs::read(&repo.data).unwrap(),before);assert!(repo.opening.lock().unwrap().is_empty());assert!(repo.cooldown.lock().unwrap().is_empty());}}
 #[test]
 fn saved_chat_open_preeffect_failure_does_not_consume_admission(){
  let (_temp,repo,id)=fixture();repo.transaction(|s|{s["sessions"][0]["sessionId"]=json!(uuid());Ok(())}).unwrap();let key=format!("open:{id}");repo.cooldown.lock().unwrap().insert(key.clone(),1.0);let before=repo.cooldown.lock().unwrap().clone();
  assert!(open_chat_with(&repo,&json!({"recordId":id}),|_,_|Err(Error::new("Invalid launch arguments"))).is_err());assert_eq!(*repo.cooldown.lock().unwrap(),before);
  let mut calls=0;let receipt=open_chat_with(&repo,&json!({"recordId":id}),|_,_|{calls+=1;Ok(terminal::OpenOutcome::Accepted)}).unwrap();assert_eq!(calls,1);assert_eq!(receipt["status"],"accepted");let again=open_chat_with(&repo,&json!({"recordId":id}),|_,_|panic!("Cooldown must not launch again")).unwrap();assert_eq!(again["status"],"unknown");
 }
 #[test]
 fn saved_chat_open_preserves_latest_metadata_and_uses_captured_context(){
  let (_temp,repo,id)=fixture();let receipt=open_chat_with(&repo,&json!({"recordId":id}),|record,path|{
   assert_eq!(record["title"],"Captured title");let context=std::fs::read_to_string(path).unwrap();assert!(context.contains("Captured title")&&context.contains("Captured next"));
   repo.transaction(|s|{let row=&mut s["sessions"][0];row["title"]=json!("Concurrent title");row["items"]=json!([{"id":"note:new-link"}]);row["checkpoint"]=json!({"next":"New next"});Ok(())}).unwrap();Ok(terminal::OpenOutcome::Accepted)
  }).unwrap();assert_eq!(receipt["status"],"accepted");assert_eq!(receipt["localSaved"],true);assert_eq!(receipt["terminal"],json!({"state":"acknowledged"}));assert!(receipt.get("generation").is_none());
  let s=repo.state.lock().unwrap();assert_eq!(s["sessions"][0]["title"],"Concurrent title");assert_eq!(s["sessions"][0]["items"][0]["id"],"note:new-link");assert_eq!(s["sessions"][0]["checkpoint"]["next"],"New next");assert_eq!(s["sessions"][0]["privateNs"]["keep"],true);assert_eq!(s["sessions"][0]["launchNew"],false);
 }
 #[test]
 fn saved_chat_open_accepted_then_save_failure_or_deleted_record_stays_accepted(){
  for delete in [false,true]{let (_temp,repo,id)=fixture();let before=std::fs::read(&repo.data).unwrap();let receipt=open_chat_with(&repo,&json!({"recordId":id}),|_,_|{
   if delete{repo.transaction(|s|{s["sessions"]=json!([]);Ok(())}).unwrap();}else{std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();}Ok(terminal::OpenOutcome::Accepted)
  }).unwrap();assert_eq!(receipt["status"],"accepted");assert_eq!(receipt["localSaved"],false);assert!(repo.opening.lock().unwrap().is_empty());if !delete{assert_eq!(std::fs::read(&repo.data).unwrap(),before);assert_eq!(repo.state.lock().unwrap()["sessions"][0]["launchNew"],true);}}
 }
 #[test]
 fn saved_chat_open_unknown_never_patches_launch_metadata(){let (_temp,repo,id)=fixture();let before=std::fs::read(&repo.data).unwrap();let mut calls=0;let result=open_chat_with(&repo,&json!({"recordId":id}),|_,_|{calls+=1;Ok(terminal::OpenOutcome::Unknown)}).unwrap();assert_eq!(calls,1);assert_eq!(result["status"],"unknown");assert_eq!(result["terminal"]["state"],"unconfirmed");assert_eq!(result["localSaved"],false);assert_eq!(std::fs::read(&repo.data).unwrap(),before);}
 #[test]
 fn saved_chat_open_snapshot_projection_excludes_private_dispatch_and_review_drafts(){let (_temp,repo,id)=fixture();repo.transaction(|s|{s["sessionDispatch"]=json!({"secret":"private-dispatch-canary"});s["sessions"][0]["workflow"]=json!("work-review");s["sessions"][0]["reviewDraft"]=json!({"note":"private-draft-canary"});Ok(())}).unwrap();let (_,public)=repo.capture_for_open().unwrap();assert_eq!(public,repo.snapshot(false));open_chat_with(&repo,&json!({"recordId":id}),|_,path|{let value=std::fs::read_to_string(path.with_extension("json")).unwrap();assert!(!value.contains("private-dispatch-canary"));assert!(!value.contains("private-draft-canary"));Ok(terminal::OpenOutcome::Preview)}).unwrap();}
 #[test]
 fn saved_chat_open_rejects_stale_record_fields_before_launch(){let (_temp,repo,id)=fixture();for args in [json!({"recordId":id,"title":"Stale"}),json!({"recordId":id,"target":"desktop"}),json!({"recordId":id,"items":[]}),json!({"recordId":"missing"})]{assert!(open_chat_with(&repo,&args,|_,_|panic!("Rejected intent must not launch")).is_err());}assert!(repo.opening.lock().unwrap().is_empty());}
}

#[cfg(test)]
mod registration_merge_tests{
 use super::*;
 fn fixture()->(tempfile::TempDir,Arc<Repository>,String,String){let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().join("profile"),None,true).unwrap();let id=uuid();let session=uuid();repo.transaction(|s|{s["sessions"]=json!([{"id":id,"sessionId":session,"agent":"codex","title":"Before","cwd":temp.path(),"instruction":"Keep instruction","destination":"embedded","launchNew":false,"items":[{"id":"note:old","source":"notes","title":"Old","linkExtension":{"keep":true}}],"preparationRequests":{"version":1,"entries":[{"requestId":uuid(),"kind":"new","hash":"f".repeat(64),"reused":false}]},"futureNs":{"keep":true}}]);s["notes"]=json!([{ "id":"note:old","source":"notes","title":"Old"},{"id":"note:concurrent","source":"notes","title":"Concurrent"}]);s["itemSettings"]["note:old"]=json!({"cwd":"existing folder","explicit":true});Ok(())}).unwrap();(temp,repo,id,session)}
 #[test]
 fn registration_resolution_merges_latest_links_and_omitted_title_without_overwriting_metadata(){let (temp,repo,id,session)=fixture();let before_alias=repo.state.lock().unwrap()["sessions"][0]["preparationRequests"].clone();let payload=json!({"id":id,"agent":"codex","sessionId":session,"cwd":temp.path(),"preserveItemFolders":true,"title":"","itemIds":[],"urls":["https://example.invalid/new"]});let result=register_resolving(&repo,&payload,|_|{repo.transaction(|s|{s["sessions"][0]["title"]=json!("Concurrent title");s["sessions"][0]["items"].as_array_mut().unwrap().push(json!({"id":"note:concurrent","source":"notes","title":"Concurrent","newExtension":9}));s["sessions"][0]["runtime"]=json!({"state":"working"});Ok(())})?;Ok(json!({"id":"github:new","source":"github","title":"Imported","url":"https://example.invalid/new"}))}).unwrap();assert_eq!(result["id"],id);assert_eq!(result["title"],"Concurrent title");assert_eq!(result["items"].as_array().unwrap().len(),3);assert_eq!(result["items"][0]["linkExtension"]["keep"],true);assert_eq!(result["items"][1]["newExtension"],9);assert_eq!(result["preparationRequests"],before_alias);assert_eq!(result["futureNs"]["keep"],true);assert_eq!(result["runtime"]["state"],"working");assert_eq!(result["instruction"],"Keep instruction");let state=repo.state.lock().unwrap();assert_eq!(state["itemSettings"]["note:old"]["cwd"],"existing folder");assert!(state["itemSettings"].get("note:concurrent").is_none());assert!(repo.embedded.inventory().is_empty());}
 #[test]
 fn registration_explicit_title_is_intentional_and_repeated_association_keeps_exact_record(){let (temp,repo,id,session)=fixture();let result=register(&repo,&json!({"agent":"codex","sessionId":session.to_uppercase(),"title":"Intentional title","cwd":temp.path(),"itemIds":["note:concurrent"]})).unwrap();assert_eq!(result["id"],id);assert_eq!(result["title"],"Intentional title");assert_eq!(result["items"][0]["linkExtension"]["keep"],true);assert_eq!(repo.state.lock().unwrap()["sessions"].as_array().unwrap().len(),1);let again=register(&repo,&json!({"agent":"codex","sessionId":session,"cwd":temp.path(),"itemIds":[]})).unwrap();assert_eq!(again["id"],id);assert_eq!(again["title"],"Intentional title");assert_eq!(again["items"].as_array().unwrap().len(),2);}
 #[test]
 fn registration_postprobe_preserves_current_mapped_folders_for_git_and_plain_directories(){for git in [false,true]{let (temp,repo,_,_)=fixture();let record=repo.state.lock().unwrap()["sessions"][0].clone();let selected=temp.path().join("selected-elsewhere");std::fs::create_dir(&selected).unwrap();let mapping=json!({"cwd":selected,"explicit":true,"futureNs":{"keep":true}});repo.transaction(|s|{s["itemSettings"]["note:old"]=mapping.clone();Ok(())}).unwrap();let metadata=json!({"path":record["cwd"],"repoPath":record["cwd"],"branch":"main"});let result=finish_registration(&repo,&record,&[],&[],if git{Some(&metadata)}else{None}).unwrap();assert_eq!(repo.state.lock().unwrap()["itemSettings"]["note:old"],mapping);assert_eq!(result["cwd"],record["cwd"]);if git{assert_eq!(result["worktrees"][0]["itemIds"],json!(["note:old"]));}}}
 #[test]
 fn registration_postprobe_does_not_overwrite_new_folder_or_recreate_deleted_record(){let (temp,repo,id,_)=fixture();let record=repo.state.lock().unwrap()["sessions"][0].clone();let next=temp.path().join("new-folder");std::fs::create_dir(&next).unwrap();repo.transaction(|s|{s["sessions"][0]["cwd"]=json!(next);s["itemSettings"]["note:old"]["cwd"]=json!(next);Ok(())}).unwrap();finish_registration(&repo,&record,&[],&[],Some(&json!({"path":temp.path(),"repoPath":temp.path(),"branch":"stale"}))).unwrap();let state=repo.state.lock().unwrap().clone();assert_eq!(state["sessions"][0]["cwd"],json!(next));assert_eq!(state["itemSettings"]["note:old"]["cwd"],json!(next));assert!(state["sessions"][0].get("worktrees").is_none());repo.transaction(|s|{s["sessions"]=json!([]);Ok(())}).unwrap();let bytes=std::fs::read(&repo.data).unwrap();assert!(finish_registration(&repo,&record,&[],&[],None).is_err());assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);assert_eq!(repo.state.lock().unwrap()["sessions"],json!([]));assert_eq!(id,record["id"]);}
}

#[cfg(test)]
mod live_unregistered_open_tests{
 use super::*;
 struct DemoCleanup{host:Arc<super::super::terminal_daemon::Host>,id:String,generation:String}
 impl Drop for DemoCleanup{fn drop(&mut self){let _=self.host.close(&self.id,&self.generation);}}
 #[test]
 fn saved_chat_open_observation_rechecks_canonical_identity_and_keeps_uncertainty_read_only(){
 for changed in ["sessionId","agent","destination","deleted","unavailable"]{let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().join("profile"),None,true).unwrap();let id=uuid();repo.transaction(|s|{s["sessions"]=json!([{"id":id,"agent":"codex","sessionId":null,"launchNew":false,"cwd":"/invalid","destination":"embedded","items":[]}]);Ok(())}).unwrap();let receipt=open_chat_observing(&repo,&json!({"recordId":id}),|_,_|panic!("Changed or uncertain runtime must never launch"),|_|{if changed=="unavailable"{return terminal::RuntimeObservation::Unavailable;}repo.transaction(|s|{match changed{"deleted"=>s["sessions"]=json!([]),"sessionId"=>s["sessions"][0][changed]=json!(uuid()),"agent"=>s["sessions"][0][changed]=json!("claude"),_=>s["sessions"][0][changed]=json!("desktop")};Ok(())}).unwrap();terminal::RuntimeObservation::Observed(uuid())}).unwrap();assert_eq!(receipt["status"],"unknown","{changed}");assert_eq!(receipt["terminal"]["state"],"unconfirmed");assert!(!repo.data.parent().unwrap().join("contexts").exists());assert!(repo.cooldown.lock().unwrap().is_empty());assert!(repo.opening.lock().unwrap().is_empty());}
 }
 #[test]
 fn saved_chat_open_observes_unregistered_live_terminal_without_launch_packet_or_cooldown(){let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().join("profile"),None,true).unwrap();let id=uuid();let record=json!({"id":id,"agent":"codex","sessionId":null,"launchNew":false,"title":"Live unregistered","cwd":"/no-longer-existing-folder","destination":"embedded","items":[]});repo.transaction(|s|{s["sessions"]=json!([record]);Ok(())}).unwrap();repo.embedded.open_demo(&record).unwrap();let generation=text(&repo.embedded.inventory()[0],"generation").to_owned();let _cleanup=DemoCleanup{host:repo.embedded.clone(),id:id.clone(),generation:generation.clone()};let result=(||->Result<()>{let before=std::fs::read(&repo.data)?;let cooldown=repo.cooldown.lock().unwrap().clone();for _ in 0..2{let receipt=open_chat_with(&repo,&json!({"recordId":id}),|_,_|panic!("Observational reuse must not launch"))?;assert_eq!(receipt["status"],"accepted");assert_eq!(receipt["terminal"]["state"],"observed");assert_eq!(receipt["terminal"]["generation"],generation);assert_eq!(receipt["localSaved"],true);}assert_eq!(std::fs::read(&repo.data)?,before);assert_eq!(*repo.cooldown.lock().unwrap(),cooldown);assert!(!repo.data.parent().unwrap().join("contexts").exists());assert_eq!(text(&repo.embedded.inventory()[0],"generation"),generation);repo.transaction(|s|{s["sessions"][0]["destination"]=json!("desktop");Ok(())})?;assert!(open_chat_with(&repo,&json!({"recordId":id}),|_,_|panic!("Desktop null ID cannot borrow embedded runtime")).is_err());Ok(())})();let close=repo.embedded.close(&id,&generation);result.unwrap();close.unwrap();assert!(open_chat_with(&repo,&json!({"recordId":id,"target":"embedded"}),|_,_|panic!("Dead runtime must not launch")).is_err());}
}

#[cfg(test)]
mod quick_folder_cleanup_tests{
 use super::*;
 use std::os::unix::fs::symlink;
 fn fixture()->(tempfile::TempDir,Arc<Repository>,PathBuf){let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().join("profile"),None,true).unwrap();repo.transaction(|_|Ok(())).unwrap();let root=temp.path().join("work/projects");(temp,repo,root)}
 fn args()->Value{json!({"agent":"codex","newFolder":"owned-project"})}
 #[test]
 fn quick_folder_canonical_failure_removes_only_created_leaf_and_immediate_retry_succeeds(){let (_temp,repo,root)=fixture();let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();repo.cooldown.lock().unwrap().insert("quick-session".into(),1.0);let cooldown=repo.cooldown.lock().unwrap().clone();let blocked=repo.data.with_extension("tmp");std::fs::create_dir(&blocked).unwrap();let error=quick_in_root(&repo,&args(),false,None,&root).unwrap_err();assert_eq!(error.status,500);assert!(!error.message.contains("retained"));assert!(!root.join("owned-project").exists());assert!(root.is_dir());assert!(root.parent().unwrap().is_dir());assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);assert_eq!(*repo.cooldown.lock().unwrap(),cooldown);std::fs::remove_dir(blocked).unwrap();let record=quick_in_root(&repo,&args(),false,None,&root).unwrap();assert!(root.join("owned-project").is_dir());assert_eq!(record["cwd"],json!(root.join("owned-project").canonicalize().unwrap()));assert!(repo.embedded.inventory().is_empty());}
 #[test]
 fn quick_folder_review_validation_precedes_creation_for_missing_or_malformed_template(){for malformed in [false,true]{let (_temp,repo,root)=fixture();let settings=repo.settings.view();repo.settings.apply(&json!({"revision":settings["revision"],"contentHash":settings["contentHash"],"patch":{"codexReviewCommand":"$fixture-review"}}),false).unwrap();let template=repo.settings.config_directory().join("workflows/pr-review-codex.md");if malformed{std::fs::write(&template,[0xff]).unwrap();}else{std::fs::remove_file(&template).unwrap();}repo.transaction(|s|{s["items"]=json!([{"id":"github:fixture","source":"github","kind":"Pull request","title":"Fixture","url":"https://github.com/example/project/pull/1","reference":"#1"}]);Ok(())}).unwrap();let mut payload=args();payload["itemId"]=json!("github:fixture");let before=std::fs::read(&repo.data).unwrap();let cooldown=repo.cooldown.lock().unwrap().clone();assert!(quick_in_root(&repo,&payload,true,None,&root).is_err());assert!(!root.exists());assert_eq!(std::fs::read(&repo.data).unwrap(),before);assert_eq!(*repo.cooldown.lock().unwrap(),cooldown);}}
 #[test]
 fn quick_folder_invalid_title_or_instruction_is_rejected_before_creation(){for invalid_instruction in [false,true]{let (_temp,repo,root)=fixture();let mut payload=args();let review=invalid_instruction;if review{let settings=repo.settings.view();repo.settings.apply(&json!({"revision":settings["revision"],"contentHash":settings["contentHash"],"patch":{"codexReviewCommand":"$fixture-review"}}),false).unwrap();std::fs::write(repo.settings.config_directory().join("workflows/pr-review-codex.md"),"x".repeat(8001)).unwrap();}repo.transaction(|s|{s["items"]=json!([{"id":"github:fixture","source":"github","kind":"Pull request","title":"Invalid\nTitle","url":"https://github.com/example/project/pull/1"}]);Ok(())}).unwrap();payload["itemId"]=json!("github:fixture");let bytes=std::fs::read(&repo.data).unwrap();assert!(quick_in_root(&repo,&payload,review,None,&root).is_err());assert!(!root.exists());assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);}}
 #[test]
 fn quick_folder_cleanup_preserves_added_content_and_original_error_details(){let temp=tempfile::tempdir().unwrap();let path=temp.path().join("owned");let mut folder=Some(CreatedQuickFolder::create(&path).unwrap());std::fs::write(path.join("someone-else.txt"),b"keep content").unwrap();let mut failure=Error::new("Original failure.");failure.status=409;failure.data=json!({"existing":"details"});failure.retry=17.0;let error=complete_quick_preparation(Err(failure),&mut folder).unwrap_err();assert_eq!(error.status,409);assert_eq!(error.data,json!({"existing":"details"}));assert_eq!(error.retry,17.0);assert!(error.message.starts_with("Original failure."));assert!(error.message.contains("retained"));assert_eq!(std::fs::read(path.join("someone-else.txt")).unwrap(),b"keep content");std::fs::remove_file(path.join("someone-else.txt")).unwrap();drop(folder);assert!(path.is_dir(),"Drop must not retry a cleanup that already chose preservation");}
 #[test]
 fn quick_folder_cleanup_preserves_replacement_inode_and_symlink_targets(){for replace_symlink in [false,true]{let temp=tempfile::tempdir().unwrap();let path=temp.path().join("owned");let original=temp.path().join("renamed-owned");let mut folder=Some(CreatedQuickFolder::create(&path).unwrap());std::fs::rename(&path,&original).unwrap();if replace_symlink{let target=temp.path().join("target");std::fs::create_dir(&target).unwrap();std::fs::write(target.join("keep"),b"target data").unwrap();symlink(&target,&path).unwrap();}else{std::fs::create_dir(&path).unwrap();}let error=complete_quick_preparation(Err(Error::new("Fixture failure")),&mut folder).unwrap_err();assert!(error.message.contains("retained"));drop(folder);assert!(original.is_dir());assert!(std::fs::symlink_metadata(&path).is_ok());if replace_symlink{assert!(std::fs::symlink_metadata(&path).unwrap().file_type().is_symlink());assert_eq!(std::fs::read(path.join("keep")).unwrap(),b"target data");}}}
 #[test]
 fn quick_folder_success_disarms_cleanup_and_preexisting_targets_are_never_owned(){let (_temp,repo,root)=fixture();std::fs::create_dir_all(&root).unwrap();let target=root.join("owned-project");std::fs::create_dir(&target).unwrap();std::fs::write(target.join("keep"),b"preexisting").unwrap();let before=std::fs::read(&repo.data).unwrap();assert!(quick_in_root(&repo,&args(),false,None,&root).is_err());assert_eq!(std::fs::read(target.join("keep")).unwrap(),b"preexisting");assert_eq!(std::fs::read(&repo.data).unwrap(),before);let path=root.join("committed");let mut folder=Some(CreatedQuickFolder::create(&path).unwrap());let request=uuid();let record=json!({"id":uuid(),"cwd":path,"preparationRequests":{"version":1,"entries":[{"requestId":request,"kind":"quick","hash":context_kit::sha256(&serde_json::to_vec(&json!({"version":1,"kind":"quick","intent":args()})).unwrap()),"reused":false}]}});repo.transaction(|s|{s["sessions"].as_array_mut().unwrap().push(record.clone());Ok(())}).unwrap();assert_eq!(complete_quick_preparation(Ok(record.clone()),&mut folder).unwrap(),record);drop(folder);assert!(path.is_dir());let later_error=Error::new("Response projection failed");drop(later_error);assert!(path.is_dir());let expected=chat_preparation::read(&repo,&json!({"requestId":request})).unwrap();let mut retry=args();retry["requestId"]=json!(request);let bytes=std::fs::read(&repo.data).unwrap();assert_eq!(chat_preparation::prepare(&repo,"quick",&retry).unwrap(),expected);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);assert!(path.is_dir());}
 #[test]
 fn quick_folder_missing_identity_is_retained_and_drop_only_removes_empty_owned_leaf(){let temp=tempfile::tempdir().unwrap();let path=temp.path().join("owned");let mut folder=CreatedQuickFolder::create(&path).unwrap();folder.identity=None;let error=complete_quick_preparation(Err(Error::new("Cannot inspect identity")),&mut Some(folder)).unwrap_err();assert!(error.message.contains("retained"));assert!(path.exists());let empty=temp.path().join("empty");{let _folder=CreatedQuickFolder::create(&empty).unwrap();}assert!(!empty.exists());assert!(temp.path().exists());}
}
