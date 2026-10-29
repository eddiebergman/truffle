//! Conservative tracked submission over the existing, generation-unfenced PTY transport.
use super::*;
const JOURNAL:&str="sessionDispatch";
const LEGACY_MAX_ENTRIES:usize=1024;
const MAX_UNRESOLVED:usize=128;
const RECENT_COMPLETED:usize=128;
const MAX_BYTES:usize=5*1024*1024;
#[derive(Clone,Copy,PartialEq,Eq)]
pub enum Kind {ReviewRequest,ReviewFeedback,CiInvestigation}
impl Kind {fn name(self)->&'static str{match self{Self::ReviewRequest=>"review-request",Self::ReviewFeedback=>"review-feedback",Self::CiInvestigation=>"ci-investigation"}}}
fn entries(state:&Value)->Result<Vec<Value>>{
    let journal=&state[JOURNAL];if journal.is_null(){return Ok(vec![]);}
    if journal["version"]!=1&&journal["version"]!=2{return Err(Error::new("Unsupported dispatch journal version. Existing terminals were kept."));}
    let limit=if journal["version"]==1{LEGACY_MAX_ENTRIES}else{LEGACY_MAX_ENTRIES+RECENT_COMPLETED};let byte_limit=if journal["version"]==1{4*1024*1024}else{MAX_BYTES};
    let entries=journal["entries"].as_array().filter(|entries|entries.len()<=limit).ok_or(Error::new("Invalid dispatch journal."))?;
    if serde_json::to_vec(journal)?.len()>byte_limit{return Err(Error::new("Dispatch journal exceeds its safe capacity. Existing terminals were kept."));}
    let mut ids=std::collections::HashSet::new();let mut tickets=std::collections::HashSet::new();
    let next=if journal["version"]==2{let next=decimal(&journal["nextTicket"])?;let retired=decimal(&journal["retiredThrough"])?;if next==0||retired>=next{return Err(Error::new("Invalid dispatch ticket metadata."));}Some(next)}else{None};
    for entry in entries {
        if !ids.insert(text(entry,"operationId")){return Err(Error::new("Duplicate dispatch operation identity."));}
        if let Some(next)=next{let ticket=decimal(&entry["ticket"])?;if ticket==0||ticket>=next||!tickets.insert(ticket){return Err(Error::new("Invalid dispatch ticket metadata."));}}
        if !["review-request","review-feedback","ci-investigation"].contains(&text(entry,"kind"))||!["prepared","sending","written","uncertain"].contains(&text(entry,"status"))||!entry["finalized"].is_boolean(){return Err(Error::new("Invalid dispatch receipt metadata."));}
        for key in ["operationId","recordId","generation"]{sid(text(entry,key))?;}
        let hash=text(entry,"payloadHash");if hash.len()!=64||!hash.bytes().all(|byte|byte.is_ascii_hexdigit()){return Err(Error::new("Invalid dispatch payload identity."));}
        if entry.get("resolved").is_some_and(|value|!value.is_boolean())||entry.get("resolution").is_some_and(|value|!value.as_str().is_some_and(|value|["user-confirmed-submitted","user-abandoned"].contains(&value))){return Err(Error::new("Invalid dispatch resolution metadata."));}
        if entry["kind"]=="ci-investigation"&&(text(entry,"itemId").is_empty()||text(entry,"itemId").len()>200){return Err(Error::new("Invalid dispatch work identity."));}
        if !closed(entry)&&entry["payload"].as_str().is_none_or(|payload|payload.len()>65536){return Err(Error::new("Invalid frozen dispatch payload."));}
    }
    Ok(entries.clone())
}
fn decimal(value:&Value)->Result<u64>{
    let text=value.as_str().ok_or(Error::new("Invalid dispatch ticket metadata."))?;
    if text.is_empty()||text.len()>20||!text.bytes().all(|byte|byte.is_ascii_digit())||(text.len()>1&&text.starts_with('0')){return Err(Error::new("Invalid dispatch ticket metadata."));}
    text.parse().map_err(|_|Error::new("Invalid dispatch ticket metadata."))
}
fn closed(entry:&Value)->bool{entry["finalized"]==true||entry["resolution"]=="user-abandoned"}
fn migrate(state:&mut Value)->Result<()>{
    let mut all=entries(state)?;
    if state[JOURNAL]["version"]==2{return Ok(());}
    if state[JOURNAL].is_null(){state[JOURNAL]=json!({});}
    if ["nextTicket","retiredThrough"].iter().any(|key|state[JOURNAL].get(*key).is_some()){return Err(Error::new("Legacy dispatch ticket extensions cannot be migrated safely. Existing data was kept."));}
    for (index,entry) in all.iter_mut().enumerate(){if entry.get("ticket").is_some(){return Err(Error::new("Legacy dispatch ticket extension cannot be migrated safely. Existing data was kept."));}entry["ticket"]=json!((index as u64+1).to_string());}
    state[JOURNAL]["version"]=json!(2);state[JOURNAL]["nextTicket"]=json!((all.len() as u64+1).to_string());state[JOURNAL]["retiredThrough"]=json!("0");state[JOURNAL]["entries"]=json!(all);
    entries(state)?;Ok(())
}
fn migrate_owned(repo:&Repository,state:&mut Value)->Result<()>{
    if state[JOURNAL]["version"]==1{entries(state)?;repo.backup_dispatch_v1(&state[JOURNAL])?;}migrate(state)
}
fn ensure_migrated(repo:&Repository)->Result<()>{repo.transaction_if_changed(|state|migrate_owned(repo,state))}
fn store(state:&mut Value,all:Vec<Value>)->Result<()>{store_entries(state,all,true)}
fn store_entries(state:&mut Value,mut all:Vec<Value>,compact:bool)->Result<()>{
    // Only receipts with a terminal user decision or completed acknowledgement may retire.
    // A retirement floor is diagnostic: unresolved tickets below it remain fully recoverable.
    let mut completed=all.iter().filter(|entry|closed(entry)).map(|entry|Ok((entry["finalizedAt"].as_f64().or(entry["resolvedAt"].as_f64()).filter(|at|at.is_finite()).unwrap_or(0.0),decimal(&entry["ticket"])?))).collect::<Result<Vec<_>>>()?;completed.sort_by(|a,b|a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let remove=completed.len().saturating_sub(RECENT_COMPLETED);
    if compact&&remove>0{let retired_tickets=completed.iter().take(remove).map(|row|row.1).collect::<std::collections::HashSet<_>>();let floor=retired_tickets.iter().copied().max().unwrap();all.retain(|entry|!closed(entry)||!retired_tickets.contains(&decimal(&entry["ticket"]).unwrap()));let retired=decimal(&state[JOURNAL]["retiredThrough"])?;state[JOURNAL]["retiredThrough"]=json!(retired.max(floor).to_string());}
    state[JOURNAL]["entries"]=json!(all);
    if serde_json::to_vec(&state[JOURNAL])?.len()>MAX_BYTES{return Err(Error::new("Tracked dispatch storage is full of retained recovery data. Resolve an earlier request; no input was sent."));}entries(state)?;Ok(())
}
fn record(state:&Value,id:&str)->Result<Value>{arr(&state["sessions"]).into_iter().find(|record|record["id"]==id).ok_or(Error::new("Session unavailable."))}
fn public(entry:&Value)->Value{
    let status=match text(entry,"status"){"prepared"=>"prepared","written"=>"written",_=>"uncertain"};
    let finalized=entry["finalized"]==true;
    let message=if entry["resolution"]=="user-confirmed-submitted"{if finalized{"You marked this request as submitted; its local acknowledgement is saved. This does not confirm agent consumption or completion."}else{"You marked this request as submitted. Save its local acknowledgement without sending again."}}else if entry["resolution"]=="user-abandoned"{"You released this request. Any uncertain delivery remains unknown. A new submission requires a separate action and may duplicate it."}else{match (status,finalized){("prepared",_)=>"Not submitted. Retry this same operation; no terminal input was sent.",("written",true)=>"Submitted to the terminal. This does not confirm the agent has read it or completed the task.",("written",false)=>"Submitted to the terminal. Save the local acknowledgement without sending again.",_=>"Terminal delivery is uncertain. Do not send again. Check the existing terminal; refreshing delivery status will not resend."}};
    let mut result=json!({"type":"operation","operationId":entry["operationId"],"recordId":entry["recordId"],"kind":entry["kind"],"generation":entry["generation"],"payloadHash":entry["payloadHash"],"status":status,"finalized":finalized,"transport":"legacy-unfenced","message":message});
    if entry.get("ticket").is_some(){result["ticket"]=entry["ticket"].clone();}
    if entry["kind"]=="ci-investigation"{result["itemId"]=entry["itemId"].clone();}if entry.get("resolved").is_some(){result["resolved"]=entry["resolved"].clone();result["resolution"]=entry["resolution"].clone();}result
}
#[cfg(test)]
fn matching(entries:&[Value],id:&str,operation:&str,kind:Kind,item:&str)->Result<Option<Value>>{
    if let Some(entry)=entries.iter().find(|entry|entry["operationId"]==operation){
        if entry["recordId"]!=id||entry["kind"]!=kind.name()||text(entry,"itemId")!=item{return Err(Error::new("This dispatch operation ID belongs to a different feature or target."));}
        return Ok(Some(entry.clone()));
    }
    Ok(entries.iter().rev().find(|entry|entry["recordId"]==id&&entry["kind"]==kind.name()&&text(entry,"itemId")==item&&entry["finalized"]!=true&&entry["resolution"]!="user-abandoned").cloned())
}
pub fn read(repo:&Repository,args:&Value)->Result<Value>{
    let id=sid(text(args,"recordId"))?;let state=repo.state.lock().map_err(|_|Error::io())?;record(&state,&id)?;let mut all=entries(&state)?.into_iter().rev().filter(|entry|entry["recordId"]==id).collect::<Vec<_>>();all.sort_by_key(|entry|entry["finalized"]==true||entry["resolution"]=="user-abandoned");let operations=all.iter().take(32).map(public).collect::<Vec<_>>();Ok(json!({"operations":operations}))
}
fn locate(repo:&Repository,id:&str,operation:&str)->Result<Value>{let state=repo.state.lock().map_err(|_|Error::io())?;entries(&state)?.into_iter().find(|entry|entry["recordId"]==id&&entry["operationId"]==operation).ok_or(Error::new("Dispatch receipt not found."))}
fn mutate(repo:&Repository,id:&str,operation:&str,update:impl FnOnce(&mut Value)->Result<()>)->Result<Value>{
    repo.transaction(|state|{migrate_owned(repo,state)?;let mut all=entries(state)?;let entry=all.iter_mut().find(|entry|entry["recordId"]==id&&entry["operationId"]==operation).ok_or(Error::new("Dispatch receipt not found."))?;update(entry)?;let result=entry.clone();store(state,all)?;Ok(result)})
}
pub fn finalize(repo:&Repository,args:&Value)->Result<Value>{let id=sid(text(args,"recordId"))?;let operation=sid(text(args,"operationId"))?;finalize_entry(repo,&id,&operation)}
fn finalize_entry(repo:&Repository,id:&str,operation:&str)->Result<Value>{
    ensure_migrated(repo)?;let entry=locate(repo,id,operation)?;if (entry["status"]!="written"&&entry["resolution"]!="user-confirmed-submitted")||entry["finalized"]==true{return Ok(public(&entry));}
    let result=repo.transaction(|state|{
        let mut all=entries(state)?;let index=all.iter().position(|entry|entry["recordId"]==id&&entry["operationId"]==operation).ok_or(Error::new("Dispatch receipt not found."))?;
        if (all[index]["status"]!="written"&&all[index]["resolution"]!="user-confirmed-submitted")||all[index]["finalized"]==true{return Ok(all[index].clone());}
        if all[index]["kind"]=="review-feedback"{super::reviews::acknowledge_dispatch(state,id,&all[index]["ack"])?;}
        all[index]["finalized"]=json!(true);all[index]["finalizedAt"]=json!(now());
        all[index].as_object_mut().unwrap().remove("payload");all[index].as_object_mut().unwrap().remove("ack");let result=all[index].clone();store(state,all)?;Ok(result)
    });
    Ok(public(&result.unwrap_or(entry)))
}
pub fn resolve(repo:&Repository,args:&Value)->Result<Value>{
    let id=sid(text(args,"recordId"))?;let operation=sid(text(args,"operationId"))?;let decision=text(args,"decision");
    if !["submitted","abandon"].contains(&decision){return Err(Error::new("Choose submitted or abandon."));}
    let entry=repo.transaction(|state|{migrate_owned(repo,state)?;let mut all=entries(state)?;let index=all.iter().position(|entry|entry["recordId"]==id&&entry["operationId"]==operation).ok_or(Error::new("Dispatch receipt not found."))?;
        if all[index]["finalized"]==true||all[index]["resolved"]==true||all[index]["status"]=="written"{return Ok(all[index].clone());}
        if decision=="submitted"&&all[index]["status"]=="prepared"{return Err(Error::new("This request was not submitted. Abandon it or retry its original operation."));}
        all[index]["resolved"]=json!(true);all[index]["resolvedAt"]=json!(now());all[index]["resolution"]=json!(if decision=="submitted"{"user-confirmed-submitted"}else{"user-abandoned"});
        if decision=="abandon"{all[index].as_object_mut().unwrap().remove("payload");all[index].as_object_mut().unwrap().remove("ack");match text(&all[index],"kind"){"review-request"=>super::reviews::cancel_dispatch(state,&id,&operation)?,"ci-investigation"=>super::session_ci::cancel_dispatch(state,&id,text(&all[index],"itemId"),&operation)?,_=>()}}
        let result=all[index].clone();store(state,all)?;Ok(result)
    })?;
    if entry["resolution"]=="user-confirmed-submitted"{finalize_entry(repo,&id,&operation)}else{Ok(public(&entry))}
}
fn safe(value:&str)->String{value.chars().filter(|c|!((*c<='\u{1f}'&&*c!='\n'&&*c!='\t')||*c=='\u{7f}')).collect()}
fn review_pending(draft:&Value,entry:&Value)->bool{entry["reviewedAt"].is_null()&&!(draft["run"]["state"]=="complete"&&!draft["run"]["inputCommentIds"].is_array()&&entry["createdAt"].as_f64().is_some_and(|at|at<=draft["run"]["completedAt"].as_f64().or(draft["run"]["updatedAt"].as_f64()).unwrap_or(0.0)))}
fn feedback(record:&Value,draft:&Value,checked:&Value)->Result<(String,Value)>{
    let comments=arr(&draft["comments"]).into_iter().filter(|entry|review_pending(draft,entry)).collect::<Vec<_>>();
    let assessments=arr(&draft["assessments"]).into_iter().filter(|entry|entry["sentAt"].is_null()&&review_pending(draft,entry)).collect::<Vec<_>>();
    let note=if draft["note"]!=draft["reviewedNote"]{text(draft,"note")}else{""};
    if comments.is_empty()&&assessments.is_empty()&&note.trim().is_empty(){return Err(Error::new("There is no pending review feedback to send."));}
    let mut lines=vec!["Please address this review of your current work. These comments are one batch. Check each against the referenced file version before changing code.".into(),format!("Working directory: {}",safe(text(record,"cwd")))];
    for assessment in &assessments {let fresh=arr(&checked["assessments"]).into_iter().find(|entry|entry["id"]==assessment["id"]).unwrap_or_else(||assessment.clone());if fresh["freshness"].is_string()&&fresh["freshness"]!="current"{lines.push(format!("Reviewer {} assessment freshness: {}. Recheck its findings against current code.",safe(text(assessment,"author")),safe(text(&fresh,"freshness"))));}}
    for comment in &comments {let fresh=arr(&checked["comments"]).into_iter().find(|entry|entry["id"]==comment["id"]).unwrap_or_else(||comment.clone());if fresh["freshness"].is_string()&&fresh["freshness"]!="current"{lines.push(format!("Comment on {}/{} freshness: {}.",safe(text(comment,"root")),safe(text(comment,"path")),safe(text(&fresh,"freshness"))));}}
    if !note.trim().is_empty(){lines.push(format!("\nOverall review:\n{}",safe(note)));}
    for entry in &assessments{let scope=if !text(entry,"root").is_empty(){format!("Worktree: {} · base: {}",safe(text(entry,"root")),safe(text(entry,"baseCommit")))}else{"Scope: conversation across repositories".into()};lines.push(format!("\nReviewer assessment · {} · {}\n{}\n{}",safe(text(entry,"author")),safe(text(entry,"pass")),scope,safe(text(entry,"text"))));}
    for (n,entry) in comments.iter().enumerate(){lines.push(format!("\n{}. {} · {} lines {}–{}\nWorktree: {}\nBase: {} · diff fingerprint: {}\nCode excerpt:\n{}\nAuthor: {}\nComment: {}",n+1,safe(text(entry,"path")),if entry["side"]=="deletions"{"old"}else{"new"},entry["start"],entry["end"],safe(entry["root"].as_str().unwrap_or(text(record,"cwd"))),safe(text(entry,"baseCommit")),safe(text(entry,"fingerprint")),safe(text(entry,"excerpt")),safe(entry["author"].as_str().unwrap_or("You")),safe(text(entry,"text"))));}
    let ack=json!({"commentIds":comments.iter().map(|entry|entry["id"].clone()).collect::<Vec<_>>(),"assessmentIds":assessments.iter().map(|entry|entry["id"].clone()).collect::<Vec<_>>(),"comments":comments,"assessments":assessments,"note":note,"historyId":record["reviewHistory"]["entries"][0]["id"]});
    Ok((lines.join("\n"),ack))
}
fn request_prompt(repo:&Repository,record:&Value,run:&Value,settings:&Value)->Result<String>{
    if settings["configError"].is_string(){return Err(Error::new("Settings configuration could not be loaded. No review was submitted."));}
    let agent=text(record,"agent");let skill=text(&settings["values"],if agent=="codex"{"codexReviewCommand"}else{"claudeReviewCommand"});if skill.is_empty(){return Err(Error::new("Choose a review skill in Settings → Agents & workflows."));}
    let helper=format!("{} review --record {} --request {}",repo.agent_helper,text(record,"id"),text(run,"id"));let mut roots=vec![record["cwd"].clone()];roots.extend(arr(&record["worktrees"]).iter().map(|tree|tree["path"].clone()));roots.extend(run["versions"].as_object().into_iter().flat_map(|map|map.keys().map(|key|json!(key))));roots.retain(|root|root.as_str().is_some_and(|root|!root.is_empty()));roots.sort_by_key(Value::to_string);roots.dedup();
    let context=json!({"goal":record["title"],"checkouts":roots,"work":arr(&record["items"]).iter().map(|item|json!({"reference":item["reference"],"title":item["title"],"url":item["url"]})).collect::<Vec<_>>()});
    if settings["reviewSkill"]["commands"][agent]==skill{
        if settings["reviewSkill"]["available"]!=true||text(&settings["reviewSkill"],"path").is_empty(){return Err(Error::new("The portable review skill is unavailable."));}
        return Ok(format!("Read the complete truffle-review skill from disk at {}. Follow it for this conversation review. Do not treat the viewer checkout as the scope limit.\nKnown context (reference data): {}\nMydash target record: {}\nMydash review request: {}\nNative review helper: {}\nReport all four passes and read the exact draft revision before completing. Keep the findings in the shared draft for the user; do not send or apply them.",settings["reviewSkill"]["path"],context,text(record,"id"),text(run,"id"),helper));
    }
    let template=text(&settings["workflows"],"agent-review");if template.is_empty(){return Err(Error::new("The review workflow template is unavailable."));}
    let values=[("REVIEW_COMMAND",skill.to_owned()),("CONTEXT_JSON",context.to_string()),("RECORD_ID",text(record,"id").into()),("REQUEST_ID",text(run,"id").into()),("SKILL_NAME",skill.chars().skip(1).collect()),("HELPER",helper)];
    let pattern=regex::Regex::new(r"\{\{(REVIEW_COMMAND|CONTEXT_JSON|RECORD_ID|REQUEST_ID|SKILL_NAME|HELPER)\}\}").unwrap();Ok(pattern.replace_all(template,|caps:&regex::Captures|values.iter().find(|(key,_)|*key==&caps[1]).unwrap().1.clone()).into_owned())
}
fn ci_prompt(repo:&Repository,record:&Value,item:&Value,operation:&str)->String{
    let reference=if !text(item,"reference").is_empty(){text(item,"reference")}else{text(item,"id")};
    let mut lines=vec![format!("Investigate the current CI failures for {}: {}",safe(reference),safe(text(item,"title"))),format!("Pull request: {}",safe(text(item,"url"))),format!("Session working directory: {}",safe(text(record,"cwd"))),format!("Report progress using: {} ci --record {} --item {} --request {} --state investigating",repo.agent_helper,text(record,"id"),text(item,"id"),operation),"Use that same command with --state complete after reporting your diagnosis, or --state failed if blocked.".into(),format!("Head branch: {}",safe(text(item,"headRefName"))),format!("Cached check summary: {}",safe(text(&item["ci"],"state")))];
    for check in arr(&item["ci"]["checks"]).iter().filter(|check|check["state"]=="failed"){lines.push(format!("- {} · {} · {}",safe(text(check,"name")),safe(text(check,"result")),safe(text(check,"url"))));}
    lines.push("Refresh the PR checks first; this cached snapshot may be stale or incomplete. Read the failing job logs, distinguish failures caused by this PR from infrastructure failures, and report the cause and a proposed fix. Investigate only; do not change code, rerun workflows, push, or post externally.".into());lines.join("\n")
}
fn scope(all:&[Value],id:&str,kind:Kind,item:&str)->Option<Value>{all.iter().rev().find(|entry|entry["recordId"]==id&&entry["kind"]==kind.name()&&text(entry,"itemId")==item&&!closed(entry)).cloned()}
pub fn prepare(repo:&Repository,kind:Kind,args:&Value)->Result<Value>{prepare_with(repo,kind,args,||Ok(repo.embedded.inventory()),uuid)}
fn prepare_with(repo:&Repository,kind:Kind,args:&Value,inventory:impl Fn()->Result<Vec<Value>>,allocate:impl Fn()->String)->Result<Value>{
    let id=sid(text(args,"recordId"))?;let item=if kind==Kind::CiInvestigation{checked(args,"itemId",200,true)?}else{String::new()};let expected_revision=if kind==Kind::CiInvestigation{None}else{Some(args["revision"].as_u64().ok_or(Error::new("Supply the captured review revision."))?)};
    let previous={let state=repo.state.lock().map_err(|_|Error::io())?;record(&state,&id)?;scope(&entries(&state)?,&id,kind,&item)};if let Some(entry)=previous{ensure_migrated(repo)?;return Ok(public(&locate(repo,&id,text(&entry,"operationId"))?));}
        let generation=inventory()?.into_iter().find(|entry|entry["recordId"]==id&&entry["live"]==true).and_then(|entry|entry["generation"].as_str().map(str::to_owned)).ok_or(Error::new("Resume this chat in Truffle before submitting this request."))?;sid(&generation)?;
        let settings=if kind==Kind::ReviewRequest{repo.settings.view()}else{Value::Null};let baseline=if kind==Kind::ReviewRequest{super::reviews::versions(repo,&id)?}else{Value::Null};let checked=if kind==Kind::ReviewFeedback{super::reviews::action(repo,&json!({"id":id,"action":"check"}))?["review"].clone()}else{Value::Null};
        let entry=repo.transaction(|state|{
            let migrated=state[JOURNAL]["version"]!=2;migrate_owned(repo,state)?;let mut all=entries(state)?;if let Some(entry)=scope(&all,&id,kind,&item){return Ok(entry);}
            if all.iter().filter(|entry|!closed(entry)).count()>=MAX_UNRESOLVED{return Err(Error::new("Too many unresolved dispatch requests. Resolve an earlier request; no input was sent."));}
            let ticket=decimal(&state[JOURNAL]["nextTicket"])?;let next=ticket.checked_add(1).ok_or(Error::new("Dispatch ticket capacity is exhausted. Existing requests remain recoverable."))?;let operation=allocate();sid(&operation)?;if all.iter().any(|entry|entry["operationId"]==operation){return Err(Error::new("Dispatch identity allocation collided. No input was sent."));}
            let record=record(state,&id)?;let draft=super::reviews::draft(&record);
            if kind!=Kind::CiInvestigation&&expected_revision!=draft["revision"].as_u64(){return Err(Error::new("The review changed. Reload it before submitting."));}
            let (prompt,ack)=match kind{
                Kind::ReviewRequest=>{let review=super::reviews::reserve_request(state,&id,&operation,expected_revision.unwrap(),&baseline)?;(request_prompt(repo,&record,&review["run"],&settings)?,Value::Null)},
                Kind::ReviewFeedback=>feedback(&record,&draft,&checked)?,
                Kind::CiInvestigation=>{let work=domain::catalog(state).remove(&item).filter(|item|item["source"]=="github"&&item["kind"]=="Pull request").ok_or(Error::new("Choose a linked GitHub pull request."))?;super::session_ci::investigation_in_state(state,&json!({"id":id,"itemId":item,"requestId":operation,"state":"queued"}))?;(ci_prompt(repo,&record,&work,&operation),Value::Null)},
            };
            let payload=format!("\u{1b}[200~{prompt}\u{1b}[201~\r");if payload.len()>65536{return Err(Error::new("This request exceeds 64 KiB. Shorten it before submitting."));}
            let entry=json!({"operationId":operation,"ticket":ticket.to_string(),"recordId":id,"kind":kind.name(),"itemId":item,"generation":generation,"payloadHash":context_kit::sha256(payload.as_bytes()),"payload":payload,"ack":ack,"sourceRevision":expected_revision,"status":"prepared","finalized":false,"createdAt":now()});all.push(entry.clone());state[JOURNAL]["nextTicket"]=json!(next.to_string());store_entries(state,all,!migrated)?;Ok(entry)
        })?;Ok(public(&entry))
}
fn expired(id:&str,operation:&str)->Value{json!({"type":"unavailable","operationId":operation,"recordId":id,"reason":"possibly-expired","transport":"legacy-unfenced","message":"This operation is unavailable or its receipt expired. Prior delivery may have occurred. No input was sent by this submission. Inspect the chat before preparing any separate new request."})}
pub fn submit(repo:&Repository,args:&Value)->Result<Value>{submit_with(repo,args,||Ok(repo.embedded.inventory()),|id,data|repo.embedded.input(id,data),||Ok(()))}
fn submit_with(repo:&Repository,args:&Value,inventory:impl Fn()->Result<Vec<Value>>,input:impl Fn(&str,&str)->Result<()>,after_written:impl FnOnce()->Result<()>)->Result<Value>{
    let id=sid(text(args,"recordId"))?;let operation=sid(text(args,"operationId"))?;ensure_migrated(repo)?;
    let prepared={let state=repo.state.lock().map_err(|_|Error::io())?;record(&state,&id)?;let all=entries(&state)?;if let Some(entry)=all.iter().find(|entry|entry["operationId"]==operation){if entry["recordId"]!=id{return Err(Error::new("This dispatch operation belongs to another chat."));}entry.clone()}else{return Ok(expired(&id,&operation));}};
    if prepared["resolution"]=="user-abandoned"{return Ok(public(&prepared));}
    if prepared["status"]=="written"||prepared["resolution"]=="user-confirmed-submitted"{return finalize_entry(repo,&id,&operation);}
    if prepared["status"]!="prepared"{return Ok(public(&prepared));}
    let payload=text(&prepared,"payload");if context_kit::sha256(payload.as_bytes())!=text(&prepared,"payloadHash"){return Err(Error::new("The frozen dispatch payload failed its integrity check. No input was sent."));}
    // This is only a best-effort preflight; the legacy daemon does not fence the write by generation.
    if !inventory()?.iter().any(|entry|entry["recordId"]==id&&entry["live"]==true&&entry["generation"]==prepared["generation"]){return Err(Error::new("The terminal generation changed. The prepared request was not sent."));}
    let sending=mutate(repo,&id,&operation,|entry|{if entry["status"]!="prepared"||entry["resolved"]==true{return Err(Error::new("Dispatch is already reserved."));}entry["status"]=json!("sending");Ok(())});
    if sending.is_err(){return Ok(public(&locate(repo,&id,&operation).unwrap_or(prepared)));}
    if input(&id,payload).is_err(){let entry=mutate(repo,&id,&operation,|entry|{entry["status"]=json!("uncertain");Ok(())}).unwrap_or_else(|_|sending.unwrap());return Ok(public(&entry));}
    let written=match mutate(repo,&id,&operation,|entry|{if entry["resolved"]==true||entry["status"]!="sending"{return Err(Error::new("Dispatch resolution changed."));}entry["status"]=json!("written");entry["writtenAt"]=json!(now());Ok(())}){Ok(entry)=>entry,Err(_)=>return Ok(public(&locate(repo,&id,&operation).unwrap_or_else(|_|sending.unwrap())))};
    if after_written().is_err(){return Ok(public(&written));}
    finalize_entry(repo,&id,&operation)
}

#[cfg(test)]
fn dispatch_with(repo:&Repository,kind:Kind,args:&Value,inventory:impl Fn()->Result<Vec<Value>>,input:impl Fn(&str,&str)->Result<()>,after_written:impl FnOnce()->Result<()>)->Result<Value>{
    let previous={let state=repo.state.lock().unwrap();matching(&entries(&state)?,text(args,"recordId"),text(args,"operationId"),kind,text(args,"itemId"))?};
    let receipt=if let Some(entry)=previous{public(&entry)}else{prepare_with(repo,kind,args,&inventory,||text(args,"operationId").to_owned())?};
    submit_with(repo,&json!({"recordId":args["recordId"],"operationId":receipt["operationId"]}),inventory,input,after_written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
    fn fixture()->(tempfile::TempDir,Arc<Repository>,Value,String){
        let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();let id=uuid();let generation=uuid();
        repo.transaction(|state|{state["sessions"]=json!([{"id":id,"agent":"codex","title":"Review fixture","cwd":temp.path(),"items":[{"id":"github:pr"}],"futureNs":{"preserve":true},"reviewDraft":{"revision":3,"note":"Captured private note","comments":[{"id":"captured","text":"Captured private comment","author":"Reviewer","path":"file.rs","root":temp.path(),"side":"additions","start":1,"end":1,"excerpt":"Captured private excerpt","createdAt":1}],"assessments":[{"id":"assessment","pass":"correctness","author":"Reviewer","text":"Captured assessment","createdAt":1}]}}]);state["items"]=json!([{"id":"github:pr","source":"github","kind":"Pull request","title":"Broken checks","url":"https://github.com/example/project/pull/1","ci":{"state":"failed","checks":[{"name":"unit","state":"failed","result":"FAIL","url":"https://example.invalid/log"}]}}]);Ok(())}).unwrap();
        let args=json!({"recordId":id,"operationId":uuid(),"revision":3});(temp,repo,args,generation)
    }
    fn inventory(args:&Value,generation:&str)->Vec<Value>{vec![json!({"recordId":args["recordId"],"generation":generation,"live":true})]}
    fn feedback_send(repo:&Repository,args:&Value,generation:&str,input:impl Fn(&str,&str)->Result<()>)->Result<Value>{dispatch_with(repo,Kind::ReviewFeedback,args,||Ok(inventory(args,generation)),input,||Ok(()))}
    #[test]
    fn dispatch_once_preserves_later_feedback_and_reused_comment_ids(){
        let (_temp,repo,args,generation)=fixture();let writes=AtomicUsize::new(0);
        let receipt=feedback_send(&repo,&args,&generation,|_,payload|{writes.fetch_add(1,Ordering::SeqCst);assert!(payload.starts_with("\u{1b}[200~"));assert!(payload.ends_with("\u{1b}[201~\r"));assert_eq!(repo.state.lock().unwrap()[JOURNAL]["entries"][0]["status"],"sending");
            repo.transaction(|s|{let draft=&mut s["sessions"][0]["reviewDraft"];draft["comments"]=json!([{"id":"captured","text":"Reused ID with newer content"},{"id":"later","text":"Later feedback"}]);draft["assessments"].as_array_mut().unwrap().push(json!({"id":"later-assessment","text":"Later assessment"}));draft["note"]=json!("Later note");draft["revision"]=json!(4);Ok(())})?;Ok(())}).unwrap();
        assert_eq!(receipt["status"],"written");assert_eq!(receipt["finalized"],true);assert_eq!(receipt["transport"],"legacy-unfenced");assert_eq!(writes.load(Ordering::SeqCst),1);
        let state=repo.state.lock().unwrap().clone();let draft=&state["sessions"][0]["reviewDraft"];assert_eq!(arr(&draft["comments"]).len(),2);assert_eq!(draft["note"],"Later note");assert!(draft["assessments"][0]["sentAt"].is_number());assert!(draft["assessments"][1]["sentAt"].is_null());assert_eq!(state["sessions"][0]["futureNs"]["preserve"],true);assert!(state[JOURNAL]["entries"][0].get("payload").is_none());
        let mut retry=args.clone();retry["revision"]=json!(999);let again=feedback_send(&repo,&retry,&generation,|_,_|panic!("written operation must never resend")).unwrap();assert_eq!(again["operationId"],receipt["operationId"]);assert_eq!(again["finalized"],true);
        assert!(repo.snapshot(false).get(JOURNAL).is_none());assert!(repo.snapshot(true).get(JOURNAL).is_none());let public=read(&repo,&args).unwrap().to_string();assert!(!public.contains("Captured private"));assert!(!public.contains("payload\""));
    }
    #[test]
    fn dispatch_uncertainty_and_failed_written_marker_survive_restart_without_replay(){
        for marker_failure in [false,true]{let (temp,repo,args,generation)=fixture();let path=repo.data.clone();let receipt=feedback_send(&repo,&args,&generation,|_,_|{if marker_failure{std::fs::create_dir(path.with_extension("tmp"))?;Ok(())}else{Err(Error::new("Partial write or lost daemon acknowledgement"))}}).unwrap();assert_eq!(receipt["status"],"uncertain");assert_eq!(receipt["finalized"],false);if marker_failure{std::fs::remove_dir(path.with_extension("tmp")).unwrap();}drop(repo);
            let repo=Repository::open(temp.path().into(),None,true).unwrap();let mut retry=args.clone();retry["operationId"]=json!(uuid());let receipt=feedback_send(&repo,&retry,&generation,|_,_|panic!("uncertain scope must not resend")).unwrap();assert_eq!(receipt["operationId"],args["operationId"]);assert_eq!(receipt["status"],"uncertain");assert_eq!(finalize(&repo,&args).unwrap()["status"],"uncertain");assert_eq!(repo.state.lock().unwrap()["sessions"][0]["reviewDraft"]["note"],"Captured private note");}
    }
    #[test]
    fn dispatch_written_marker_allows_only_ack_recovery_after_failed_save_and_restart(){
        let (temp,repo,args,generation)=fixture();let path=repo.data.clone();let receipt=dispatch_with(&repo,Kind::ReviewFeedback,&args,||Ok(inventory(&args,&generation)),|_,_|Ok(()),||{std::fs::create_dir(path.with_extension("tmp"))?;Ok(())}).unwrap();assert_eq!(receipt["status"],"written");assert_eq!(receipt["finalized"],false);assert_eq!(repo.state.lock().unwrap()["sessions"][0]["reviewDraft"]["note"],"Captured private note");std::fs::remove_dir(path.with_extension("tmp")).unwrap();drop(repo);
        let repo=Repository::open(temp.path().into(),None,true).unwrap();let receipt=feedback_send(&repo,&args,&generation,|_,_|panic!("ACK recovery must not send")).unwrap();assert_eq!(receipt["finalized"],true);assert_eq!(repo.state.lock().unwrap()["sessions"][0]["reviewDraft"]["note"],"");assert_eq!(arr(&repo.state.lock().unwrap()["sessions"][0]["reviewDraft"]["comments"]).len(),0);
    }
    #[test]
    fn dispatch_prepared_retry_uses_frozen_payload_and_reservation_failure_never_queues(){
        let (_temp,repo,args,generation)=fixture();let inventories=AtomicUsize::new(0);let path=repo.data.clone();let first=dispatch_with(&repo,Kind::ReviewFeedback,&args,||{if inventories.fetch_add(1,Ordering::SeqCst)==1{std::fs::create_dir(path.with_extension("tmp"))?;}Ok(inventory(&args,&generation))},|_,_|panic!("failed sending marker must prevent I/O"),||Ok(())).unwrap();assert_eq!(first["status"],"prepared");std::fs::remove_dir(path.with_extension("tmp")).unwrap();repo.transaction(|s|{s["sessions"][0]["title"]=json!("New title");s["sessions"][0]["reviewDraft"]["note"]=json!("New note");s["sessions"][0]["reviewDraft"]["revision"]=json!(4);Ok(())}).unwrap();let receipt=feedback_send(&repo,&args,&generation,|_,payload|{assert!(payload.contains("Captured private note"));assert!(!payload.contains("New note"));Ok(())}).unwrap();assert_eq!(receipt["payloadHash"],first["payloadHash"]);assert_eq!(repo.state.lock().unwrap()["sessions"][0]["reviewDraft"]["note"],"New note");
        let (_temp,repo,mut args,generation)=fixture();args["itemId"]=json!("github:pr");std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();let before=repo.state.lock().unwrap().clone();assert!(dispatch_with(&repo,Kind::CiInvestigation,&args,||Ok(inventory(&args,&generation)),|_,_|panic!("reservation failed"),||Ok(())).is_err());assert_eq!(*repo.state.lock().unwrap(),before);assert!(repo.state.lock().unwrap()["sessions"][0]["ciInvestigations"].is_null());
    }
    #[test]
    fn dispatch_concurrent_same_operation_has_only_one_terminal_write(){
        let (_temp,repo,args,generation)=fixture();let (started,ready)=std::sync::mpsc::channel();let (release,wait)=std::sync::mpsc::channel();let wait=std::sync::Mutex::new(wait);let worker_repo=repo.clone();let worker_args=args.clone();let worker_generation=generation.clone();let worker=std::thread::spawn(move||feedback_send(&worker_repo,&worker_args,&worker_generation,|_,_|{started.send(()).unwrap();wait.lock().unwrap().recv().unwrap();Ok(())}));ready.recv().unwrap();let receipt=feedback_send(&repo,&args,&generation,|_,_|panic!("concurrent operation duplicated input")).unwrap();assert_eq!(receipt["status"],"uncertain");release.send(()).unwrap();assert_eq!(worker.join().unwrap().unwrap()["finalized"],true);
    }
    #[test]
    fn dispatch_resolution_is_explicit_retains_uncertainty_and_never_replays_old_ids(){
        for decision in ["submitted","abandon"]{let (_temp,repo,args,generation)=fixture();let receipt=feedback_send(&repo,&args,&generation,|_,_|Err(Error::new("Unknown delivery"))).unwrap();assert_eq!(receipt["status"],"uncertain");let result=resolve(&repo,&json!({"recordId":args["recordId"],"operationId":args["operationId"],"decision":decision})).unwrap();assert_eq!(result["status"],"uncertain");assert_eq!(result["resolved"],true);assert_eq!(result["finalized"],decision=="submitted");assert_eq!(feedback_send(&repo,&args,&generation,|_,_|panic!("resolved ID must not replay")).unwrap()["resolved"],true);if decision=="abandon"{let mut next=args.clone();next["operationId"]=json!(uuid());let receipt=feedback_send(&repo,&next,&generation,|_,_|Ok(())).unwrap();assert_eq!(receipt["status"],"written");}}
    }
    #[test]
    fn dispatch_identity_revision_capacity_and_private_extensions_are_guarded(){
        let (_temp,repo,args,generation)=fixture();let mut bad=args.clone();bad.as_object_mut().unwrap().remove("revision");assert!(feedback_send(&repo,&bad,&generation,|_,_|panic!("missing revision")).is_err());bad["revision"]=json!(2);assert!(feedback_send(&repo,&bad,&generation,|_,_|panic!("stale revision")).is_err());assert!(entries(&repo.state.lock().unwrap()).unwrap().is_empty());
        feedback_send(&repo,&args,&generation,|_,_|Err(Error::new("Unknown"))).unwrap();let mut collision=args.clone();collision["itemId"]=json!("github:pr");assert!(dispatch_with(&repo,Kind::CiInvestigation,&collision,||panic!("identity collision before inventory"),|_,_|panic!("identity collision"),||Ok(())).is_err());
        repo.transaction(|s|{s[JOURNAL]["privateNs"]=json!({"secret":"private-journal-extension"});s[JOURNAL]["entries"][0]["futureNs"]=json!({"keep":true});Ok(())}).unwrap();resolve(&repo,&json!({"recordId":args["recordId"],"operationId":args["operationId"],"decision":"submitted"})).unwrap();let state=repo.state.lock().unwrap().clone();assert_eq!(state[JOURNAL]["entries"][0]["futureNs"]["keep"],true);assert_eq!(state[JOURNAL]["privateNs"]["secret"],"private-journal-extension");assert!(!read(&repo,&args).unwrap().to_string().contains("private-journal-extension"));assert!(repo.snapshot(true).get(JOURNAL).is_none());
        repo.transaction(|s|{let template=s[JOURNAL]["entries"][0].clone();let mut all=vec![];for index in 0..MAX_UNRESOLVED{let mut entry=template.clone();entry["operationId"]=json!(uuid());entry["recordId"]=json!(uuid());entry["ticket"]=json!((index+1).to_string());entry["finalized"]=json!(false);entry["status"]=json!("uncertain");entry.as_object_mut().unwrap().remove("resolved");entry.as_object_mut().unwrap().remove("resolution");entry["payload"]=json!("frozen");all.push(entry);}s[JOURNAL]["nextTicket"]=json!((MAX_UNRESOLVED+1).to_string());store(s,all)}).unwrap();let before=repo.state.lock().unwrap().clone();let mut next=args.clone();next["operationId"]=json!(uuid());assert!(feedback_send(&repo,&next,&generation,|_,_|panic!("journal full")).is_err());assert_eq!(*repo.state.lock().unwrap(),before);
    }
    #[test]
    fn dispatch_prompts_preserve_anchors_freshness_and_strip_terminal_controls(){
        let record=json!({"id":uuid(),"cwd":"/project","title":"Goal","items":[]});let comment=json!({"id":"c","author":"Reviewer","text":"Fix\r\u{1b}[31m\u{7}this\nnext\tcolumn","root":"/other","path":"file.rs","side":"deletions","start":2,"end":3,"baseCommit":"base123","fingerprint":"fingerprint123","excerpt":"old code","freshness":"outdated"});let assessment=json!({"id":"a","author":"Performance","pass":"performance","text":"Slow path","root":"/other","baseCommit":"otherbase","freshness":"unverified"});let draft=json!({"comments":[comment],"assessments":[assessment],"note":"Summary","revision":1});let (prompt,ack)=feedback(&record,&draft,&draft).unwrap();for fragment in ["/other/file.rs freshness: outdated","assessment freshness: unverified","Worktree: /other · base: otherbase","old lines 2–3","Base: base123 · diff fingerprint: fingerprint123","Code excerpt:\nold code","Fix[31mthis\nnext\tcolumn"]{assert!(prompt.contains(fragment),"missing {fragment}");}assert!(!prompt.contains('\r'));assert!(!prompt.contains('\u{1b}'));assert_eq!(ack["commentIds"],json!(["c"]));
        let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();let item=json!({"id":"github:pr","reference":"#7","title":"CI","url":"https://github.com/example/repo/pull/7","headRefName":"feature","ci":{"state":"failed","checks":[{"name":"unit","state":"failed","result":"FAIL","url":"https://example.invalid/log"}]}});let prompt=ci_prompt(&repo,&record,&item,"operation");for fragment in ["unit · FAIL · https://example.invalid/log","cached snapshot may be stale","Investigate only","do not change code, rerun workflows, push, or post externally","--request operation"]{assert!(prompt.contains(fragment));}
        let settings=json!({"values":{"codexReviewCommand":"$custom"},"workflows":{"agent-review":"{{REVIEW_COMMAND}} {{CONTEXT_JSON}} {{RECORD_ID}} {{REQUEST_ID}} {{HELPER}}"}});let mut record=record;record["agent"]=json!("codex");record["title"]=json!("{{HELPER}} is reference data");let run=json!({"id":uuid(),"versions":{"/another":"stamp"}});let prompt=request_prompt(&repo,&record,&run,&settings).unwrap();assert!(prompt.contains("$custom"));assert!(prompt.contains("{{HELPER}} is reference data"));assert!(prompt.contains("/another"));assert!(prompt.contains(text(&run,"id")));
    }    #[test]
    fn dispatch_request_and_ci_queue_are_atomic_and_resolution_cancels_only_matching_progress(){
        let (_temp,repo,args,generation)=fixture();let settings=repo.settings.view();let command=text(&settings["reviewSkill"]["commands"],"codex");assert!(!command.is_empty());repo.settings.apply(&json!({"revision":settings["revision"],"contentHash":settings["contentHash"],"patch":{"codexReviewCommand":command}}),false).unwrap();
        let result=dispatch_with(&repo,Kind::ReviewRequest,&args,||Ok(inventory(&args,&generation)),|_,payload|{assert!(payload.contains("truffle-review"));assert!(payload.contains(text(&args,"operationId")));let state=repo.state.lock().unwrap();assert_eq!(state["sessions"][0]["reviewDraft"]["run"]["state"],"requested");assert_eq!(state[JOURNAL]["entries"][0]["status"],"sending");Err(Error::new("Lost acknowledgement"))},||Ok(())).unwrap();assert_eq!(result["status"],"uncertain");resolve(&repo,&json!({"recordId":args["recordId"],"operationId":args["operationId"],"decision":"abandon"})).unwrap();assert_eq!(repo.state.lock().unwrap()["sessions"][0]["reviewDraft"]["run"]["state"],"cancelled");
        let (_temp,repo,mut args,generation)=fixture();args["itemId"]=json!("github:pr");let result=dispatch_with(&repo,Kind::CiInvestigation,&args,||Ok(inventory(&args,&generation)),|_,payload|{assert!(payload.contains("--request "));assert!(payload.contains("Investigate only"));assert_eq!(repo.state.lock().unwrap()["sessions"][0]["ciInvestigations"]["github:pr"]["state"],"queued");Err(Error::new("Unknown delivery"))},||Ok(())).unwrap();assert_eq!(result["status"],"uncertain");resolve(&repo,&json!({"recordId":args["recordId"],"operationId":args["operationId"],"decision":"abandon"})).unwrap();assert_eq!(repo.state.lock().unwrap()["sessions"][0]["ciInvestigations"]["github:pr"]["state"],"cancelled");let mut next=args.clone();next["operationId"]=json!(uuid());assert_eq!(dispatch_with(&repo,Kind::CiInvestigation,&next,||Ok(inventory(&args,&generation)),|_,_|Ok(()),||Ok(())).unwrap()["status"],"written");assert_eq!(dispatch_with(&repo,Kind::CiInvestigation,&args,||panic!("abandoned operation must not need inventory"),|_,_|panic!("abandoned operation must not send"),||Ok(())).unwrap()["resolution"],"user-abandoned");
    }
    #[test]
    fn dispatch_abandoned_prepared_generation_change_and_new_review_round_never_replay_or_ack_new_data(){
        let (_temp,repo,args,generation)=fixture();let count=AtomicUsize::new(0);assert!(dispatch_with(&repo,Kind::ReviewFeedback,&args,||{let generation=if count.fetch_add(1,Ordering::SeqCst)==0{generation.clone()}else{uuid()};Ok(inventory(&args,&generation))},|_,_|panic!("generation preflight must prevent input"),||Ok(())).is_err());let resolved=resolve(&repo,&json!({"recordId":args["recordId"],"operationId":args["operationId"],"decision":"abandon"})).unwrap();assert_eq!(resolved["status"],"prepared");assert_eq!(resolved["resolution"],"user-abandoned");assert!(!text(&resolved,"message").contains("Retry this same"));assert_eq!(feedback_send(&repo,&args,&generation,|_,_|panic!("abandoned prepared ID must not send")).unwrap()["resolved"],true);
        let (_temp,repo,args,generation)=fixture();let result=feedback_send(&repo,&args,&generation,|_,_|{repo.transaction(|s|{s["sessions"][0]["reviewHistory"]=json!({"version":1,"entries":[{"id":uuid()}]});Ok(())})?;Ok(())}).unwrap();assert_eq!(result["finalized"],true);assert_eq!(repo.state.lock().unwrap()["sessions"][0]["reviewDraft"]["note"],"Captured private note");
    }
    #[test]
    fn dispatch_pending_filters_and_both_agent_skill_paths_preserve_protocol_context(){
        let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();let id=uuid();let record=json!({"id":id,"cwd":"/checkout","title":"Goal","items":[{"reference":"#5","title":"Task","url":"https://example.invalid/work"}],"worktrees":[{"path":"/linked"}]});
        let draft=json!({"revision":1,"note":"Already reviewed summary","reviewedNote":"Already reviewed summary","comments":[{"id":"reviewed","text":"Consumed comment","reviewedAt":5},{"id":"pending","text":"Pending comment","path":"p","start":1,"end":1}],"assessments":[{"id":"sent","text":"Sent assessment","sentAt":5},{"id":"reviewed-assessment","text":"Consumed assessment","reviewedAt":5},{"id":"pending-assessment","text":"Pending assessment"}]});let (prompt,ack)=feedback(&record,&draft,&draft).unwrap();assert!(prompt.contains("Pending comment"));assert!(prompt.contains("Pending assessment"));for omitted in ["Consumed comment","Sent assessment","Consumed assessment","Already reviewed summary"]{assert!(!prompt.contains(omitted));}assert_eq!(ack["commentIds"],json!(["pending"]));assert_eq!(ack["assessmentIds"],json!(["pending-assessment"]));assert_eq!(ack["note"],"");
        let old=json!({"note":"","run":{"state":"complete","completedAt":10},"comments":[{"id":"old","createdAt":5,"text":"Old round"},{"id":"new","createdAt":11,"text":"After round"}]});let (prompt,_)=feedback(&record,&old,&old).unwrap();assert!(!prompt.contains("Old round"));assert!(prompt.contains("After round"));
        for (agent,command) in [("codex","$portable"),("claude","/portable")]{let mut record=record.clone();record["agent"]=json!(agent);let run=json!({"id":uuid(),"versions":{"/extra":"stamp"}});let settings=json!({"values":{if agent=="codex"{"codexReviewCommand"}else{"claudeReviewCommand"}:command},"reviewSkill":{"available":true,"path":"/editable/config/review/SKILL.md","commands":{agent:command}}});let prompt=request_prompt(&repo,&record,&run,&settings).unwrap();for fragment in ["/editable/config/review/SKILL.md","/linked","/extra","https://example.invalid/work",text(&run,"id"),"Report all four passes","do not send or apply them"]{assert!(prompt.contains(fragment),"missing {fragment}");}}
    }
    #[test]
    fn dispatch_hash_target_and_malformed_journal_cannot_admit_terminal_input(){
        let (_temp,repo,args,generation)=fixture();let count=AtomicUsize::new(0);assert!(dispatch_with(&repo,Kind::ReviewFeedback,&args,||Ok(inventory(&args,if count.fetch_add(1,Ordering::SeqCst)==0{&generation}else{"00000000-0000-0000-0000-000000000000"})),|_,_|panic!("changed generation"),||Ok(())).is_err());repo.transaction(|s|{s[JOURNAL]["entries"][0]["payload"]=json!("tampered bytes");Ok(())}).unwrap();assert!(feedback_send(&repo,&args,&generation,|_,_|panic!("hash mismatch")).unwrap_err().message.contains("integrity"));
        let other=uuid();repo.transaction(|s|{s["sessions"].as_array_mut().unwrap().push(json!({"id":other,"items":[]}));Ok(())}).unwrap();let mut collision=args.clone();collision["recordId"]=json!(other);assert!(feedback_send(&repo,&collision,&generation,|_,_|panic!("cross-record ID collision")).is_err());
        repo.transaction(|s|{s[JOURNAL]["entries"][0]["resolved"]=json!({"private":"not a boolean"});Ok(())}).unwrap();assert!(read(&repo,&args).is_err());assert!(feedback_send(&repo,&args,&generation,|_,_|panic!("malformed journal")).is_err());
    }
    #[test]
    fn dispatch_concurrent_prepare_and_lost_response_reserve_one_server_identity_without_input(){
        let (_temp,repo,args,generation)=fixture();let (started,ready)=std::sync::mpsc::channel();let (release,wait)=std::sync::mpsc::channel();let wait=std::sync::Mutex::new(wait);let worker_repo=repo.clone();let worker_args=args.clone();let worker_generation=generation.clone();
        let worker=std::thread::spawn(move||prepare_with(&worker_repo,Kind::ReviewFeedback,&worker_args,||{started.send(()).unwrap();wait.lock().unwrap().recv().unwrap();Ok(inventory(&worker_args,&worker_generation))},uuid));
        ready.recv().unwrap();assert_eq!(read(&repo,&args).unwrap()["operations"],json!([]));let result=prepare_with(&repo,Kind::ReviewFeedback,&args,||Ok(inventory(&args,&generation)),uuid).unwrap();assert_ne!(result["operationId"],args["operationId"]);release.send(()).unwrap();let recovered=worker.join().unwrap().unwrap();assert_eq!(result,recovered);assert_eq!(result["status"],"prepared");assert_eq!(result["ticket"],"1");assert_eq!(repo.state.lock().unwrap()[JOURNAL]["nextTicket"],"2");let again=prepare_with(&repo,Kind::ReviewFeedback,&args,||panic!("lost prepare must use durable reservation"),uuid).unwrap();assert_eq!(again,result);
    }

    #[test]
    fn dispatch_v2_compacts_seeded_history_and_continues_durable_cycles(){
        let (temp,repo,args,generation)=fixture();
        let unresolved=prepare_with(&repo,Kind::ReviewFeedback,&args,||Ok(inventory(&args,&generation)),uuid).unwrap();
        let other=uuid();repo.transaction(|state|{let mut record=state["sessions"][0].clone();record["id"]=json!(other);state["sessions"].as_array_mut().unwrap().push(record);Ok(())}).unwrap();
        let prepare_args=json!({"recordId":other,"itemId":"github:pr"});let writes=AtomicUsize::new(0);
        let first=prepare_with(&repo,Kind::CiInvestigation,&prepare_args,||Ok(inventory(&prepare_args,&generation)),uuid).unwrap();
        let completed=submit_with(&repo,&first,||Ok(inventory(&prepare_args,&generation)),|_,_|{writes.fetch_add(1,Ordering::SeqCst);Ok(())},||Ok(())).unwrap();
        assert_eq!(completed["finalized"],true);assert_eq!(completed["ticket"],"2");
        super::super::session_ci::investigation(&repo,&json!({"id":other,"itemId":"github:pr","requestId":first["operationId"],"state":"complete"})).unwrap();
        let seeded_completed=1030u64;
        repo.transaction(|state|{
            let mut all=entries(state)?;let template=all.iter().find(|entry|entry["operationId"]==first["operationId"]).unwrap().clone();
            let at=template["finalizedAt"].as_f64().unwrap();all.retain(|entry|entry["operationId"]!=first["operationId"]);
            for ticket in 2..=seeded_completed+1{let mut entry=template.clone();if ticket!=2{entry["operationId"]=json!(uuid());}entry["ticket"]=json!(ticket.to_string());entry["finalizedAt"]=json!(at-(seeded_completed+2-ticket) as f64);all.push(entry);}
            state[JOURNAL]["nextTicket"]=json!((seeded_completed+2).to_string());store(state,all)
        }).unwrap();
        for cycle in 0..3{
            let prepared=prepare_with(&repo,Kind::CiInvestigation,&prepare_args,||Ok(inventory(&prepare_args,&generation)),uuid).unwrap();assert_eq!(prepared["ticket"],(seeded_completed+2+cycle).to_string());
            let done=submit_with(&repo,&prepared,||Ok(inventory(&prepare_args,&generation)),|_,_|{writes.fetch_add(1,Ordering::SeqCst);Ok(())},||Ok(())).unwrap();assert_eq!(done["finalized"],true);
            assert_eq!(locate(&repo,&other,text(&prepared,"operationId")).unwrap()["finalized"],true);
            super::super::session_ci::investigation(&repo,&json!({"id":other,"itemId":"github:pr","requestId":prepared["operationId"],"state":"complete"})).unwrap();
        }
        assert_eq!(writes.load(Ordering::SeqCst),4);
        let state=repo.state.lock().unwrap().clone();assert_eq!(entries(&state).unwrap().len(),RECENT_COMPLETED+1);assert_eq!(decimal(&state[JOURNAL]["retiredThrough"]).unwrap(),seeded_completed+4-RECENT_COMPLETED as u64);assert_eq!(state[JOURNAL]["nextTicket"],(seeded_completed+5).to_string());assert_eq!(locate(&repo,text(&args,"recordId"),text(&unresolved,"operationId")).unwrap()["ticket"],"1");drop(repo);
        let repo=Repository::open(temp.path().into(),None,true).unwrap();assert_eq!(repo.state.lock().unwrap()[JOURNAL],state[JOURNAL]);let bytes=std::fs::read(&repo.data).unwrap();
        let retired=submit_with(&repo,&first,||panic!("retired operation must not inspect terminal"),|_,_|panic!("retired operation must not write"),||Ok(())).unwrap();assert_eq!(retired["type"],"unavailable");assert!(retired.get("status").is_none());assert!(text(&retired,"message").contains("Prior delivery may have occurred"));assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);
        let still_live=submit_with(&repo,&unresolved,||Ok(inventory(&args,&generation)),|_,_|{writes.fetch_add(1,Ordering::SeqCst);Err(Error::new("Unknown acknowledgement"))},||Ok(())).unwrap();assert_eq!(still_live["status"],"uncertain");assert_eq!(still_live["operationId"],unresolved["operationId"]);
        assert_eq!(submit_with(&repo,&unresolved,||panic!("uncertain operation must not inspect terminal"),|_,_|panic!("uncertain operation must not replay"),||Ok(())).unwrap(),still_live);assert_eq!(writes.load(Ordering::SeqCst),5);
    }
    fn legacy_fixture()->(tempfile::TempDir,Arc<Repository>,Value,String,Value){
        let (temp,repo,args,generation)=fixture();let prepared=prepare_with(&repo,Kind::ReviewFeedback,&args,||Ok(inventory(&args,&generation)),uuid).unwrap();repo.transaction(|s|{s[JOURNAL]["version"]=json!(1);for key in ["nextTicket","retiredThrough"]{s[JOURNAL].as_object_mut().unwrap().remove(key);}s[JOURNAL]["rootNs"]=json!({"keep":"private extension"});for entry in s[JOURNAL]["entries"].as_array_mut().unwrap(){entry.as_object_mut().unwrap().remove("ticket");entry["unknownNs"]=json!({"keep":true});}Ok(())}).unwrap();(temp,repo,args,generation,prepared)
    }
    #[test]
    fn dispatch_v2_migration_is_atomic_preserves_legacy_fields_and_recovers_existing_uuid(){
        let (_temp,repo,args,generation,prepared)=legacy_fixture();let original=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();let public_before=read(&repo,&args).unwrap();assert_eq!(public_before["operations"][0]["operationId"],prepared["operationId"]);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);
        std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();assert!(submit_with(&repo,&prepared,||panic!("migration must precede input"),|_,_|panic!("migration failed"),||Ok(())).is_err());assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);assert_eq!(*repo.state.lock().unwrap(),original);std::fs::remove_dir(repo.data.with_extension("tmp")).unwrap();
        ensure_migrated(&repo).unwrap();let migrated=repo.state.lock().unwrap().clone();let mut expected=original[JOURNAL]["entries"][0].clone();expected["ticket"]=json!("1");assert_eq!(migrated[JOURNAL]["entries"][0],expected);assert_eq!(migrated[JOURNAL]["rootNs"],original[JOURNAL]["rootNs"]);assert_eq!(migrated[JOURNAL]["version"],2);assert_eq!(migrated[JOURNAL]["nextTicket"],"2");let done=submit_with(&repo,&prepared,||Ok(inventory(&args,&generation)),|_,payload|{assert!(payload.contains("Captured private note"));Ok(())},||Ok(())).unwrap();assert_eq!(done["finalized"],true);assert_eq!(repo.state.lock().unwrap()[JOURNAL]["entries"][0]["unknownNs"]["keep"],true);
    }
    #[test]
    fn dispatch_v2_ticket_metadata_overflow_and_unknown_identity_fail_closed(){
        let (_temp,repo,args,generation)=fixture();let unknown=submit_with(&repo,&args,||panic!("unknown ID needs no inventory"),|_,_|panic!("unknown ID cannot create"),||Ok(())).unwrap();assert_eq!(unknown["type"],"unavailable");let clean=repo.state.lock().unwrap()[JOURNAL].clone();
        for next in [json!(0),json!("0"),json!("01"),json!("18446744073709551616"),json!("18446744073709551615")]{repo.transaction(|s|{s[JOURNAL]=clean.clone();s[JOURNAL]["nextTicket"]=next;Ok(())}).unwrap();let before=repo.state.lock().unwrap().clone();assert!(prepare_with(&repo,Kind::ReviewFeedback,&args,||Ok(inventory(&args,&generation)),uuid).is_err());assert_eq!(*repo.state.lock().unwrap(),before);}
        repo.transaction(|s|{s[JOURNAL]=clean.clone();s[JOURNAL]["nextTicket"]=json!("9007199254740993");Ok(())}).unwrap();let operation=prepare_with(&repo,Kind::ReviewFeedback,&args,||Ok(inventory(&args,&generation)),uuid).unwrap();assert_eq!(operation["ticket"],"9007199254740993");assert_eq!(repo.state.lock().unwrap()[JOURNAL]["nextTicket"],"9007199254740994");let good=repo.state.lock().unwrap()[JOURNAL].clone();
        for mutation in ["floor","duplicate-ticket","duplicate-id","ticket-next","ticket-number"]{repo.transaction(|s|{s[JOURNAL]=good.clone();match mutation{"floor"=>s[JOURNAL]["retiredThrough"]=s[JOURNAL]["nextTicket"].clone(),"ticket-next"=>s[JOURNAL]["entries"][0]["ticket"]=s[JOURNAL]["nextTicket"].clone(),"ticket-number"=>s[JOURNAL]["entries"][0]["ticket"]=json!(9),_=>{let mut entry=s[JOURNAL]["entries"][0].clone();if mutation=="duplicate-ticket"{entry["operationId"]=json!(uuid());}else{entry["ticket"]=json!("1");}s[JOURNAL]["entries"].as_array_mut().unwrap().push(entry);}}Ok(())}).unwrap();assert!(read(&repo,&args).is_err(),"{mutation}");}
    }
    #[test]
    fn dispatch_v2_abandon_releases_owned_bytes_and_preserves_unknown_extensions(){
        let (_temp,repo,args,generation)=fixture();let prepared=prepare_with(&repo,Kind::ReviewFeedback,&args,||Ok(inventory(&args,&generation)),uuid).unwrap();repo.transaction(|s|{s[JOURNAL]["entries"][0]["extension"]=json!({"keep":"unknown"});Ok(())}).unwrap();let resolved=resolve(&repo,&json!({"recordId":args["recordId"],"operationId":prepared["operationId"],"decision":"abandon"})).unwrap();assert_eq!(resolved["status"],"prepared");let entry=repo.state.lock().unwrap()[JOURNAL]["entries"][0].clone();assert!(entry.get("payload").is_none());assert!(entry.get("ack").is_none());assert_eq!(entry["extension"]["keep"],"unknown");assert_eq!(submit_with(&repo,&prepared,||panic!("abandoned"),|_,_|panic!("abandoned"),||Ok(())).unwrap()["resolution"],"user-abandoned");
    }

    #[test]
    fn dispatch_v2_legacy_prepare_migrates_without_reallocation_and_extension_collisions_preserve_bytes(){
        for status in ["prepared","uncertain"]{let (_temp,repo,args,_generation,old)=legacy_fixture();repo.transaction(|s|{s[JOURNAL]["entries"][0]["status"]=json!(status);Ok(())}).unwrap();let prepared=prepare_with(&repo,Kind::ReviewFeedback,&args,||panic!("existing legacy operation needs no inventory"),||panic!("existing legacy operation must not reallocate")).unwrap();assert_eq!(prepared["type"],"operation");assert_eq!(prepared["ticket"],"1");assert_eq!(prepared["status"],status);assert_eq!(prepared["operationId"],old["operationId"]);assert_eq!(repo.state.lock().unwrap()[JOURNAL]["nextTicket"],"2");}
        for key in ["nextTicket","retiredThrough","entry-ticket"]{let (_temp,repo,_args,_generation,_old)=legacy_fixture();repo.transaction(|s|{if key=="entry-ticket"{s[JOURNAL]["entries"][0]["ticket"]=json!({"legacy":"keep"});}else{s[JOURNAL][key]=json!({"legacy":"keep"});}Ok(())}).unwrap();let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();assert!(ensure_migrated(&repo).is_err());assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);}
    }

    #[test]
    fn dispatch_v2_small_compaction_keeps_old_live_and_newly_completed_receipts(){
        let (_temp,repo,args,generation)=fixture();let live=prepare_with(&repo,Kind::ReviewFeedback,&args,||Ok(inventory(&args,&generation)),uuid).unwrap();
        repo.transaction(|s|{let template=s[JOURNAL]["entries"][0].clone();let mut all=vec![template.clone()];for index in 2..=RECENT_COMPLETED+2{let mut entry=template.clone();entry["operationId"]=json!(uuid());entry["ticket"]=json!(index.to_string());entry["status"]=json!("written");entry["finalized"]=json!(true);entry["finalizedAt"]=json!(index as f64);all.push(entry);}s[JOURNAL]["nextTicket"]=json!((RECENT_COMPLETED+3).to_string());store(s,all)}).unwrap();
        let state=repo.state.lock().unwrap().clone();assert_eq!(entries(&state).unwrap().len(),RECENT_COMPLETED+1);assert!(decimal(&state[JOURNAL]["retiredThrough"]).unwrap()>1);assert_eq!(locate(&repo,text(&args,"recordId"),text(&live,"operationId")).unwrap()["status"],"prepared");
        let done=submit_with(&repo,&live,||Ok(inventory(&args,&generation)),|_,_|Ok(()),||Ok(())).unwrap();assert_eq!(done["finalized"],true);assert_eq!(entries(&repo.state.lock().unwrap()).unwrap().len(),RECENT_COMPLETED);assert_eq!(locate(&repo,text(&args,"recordId"),text(&live,"operationId")).unwrap()["finalized"],true);
    }
    #[test]
    fn dispatch_v2_initial_migration_preserves_all_legacy_receipts_before_later_compaction(){
        let (_temp,repo,args,generation,_old)=legacy_fixture();let original=repo.transaction(|s|{let template=s[JOURNAL]["entries"][0].clone();let mut all=vec![];for _ in 0..RECENT_COMPLETED+2{let mut entry=template.clone();entry["operationId"]=json!(uuid());entry["finalized"]=json!(true);entry["status"]=json!("written");all.push(entry);}s[JOURNAL]["entries"]=json!(all);Ok(s[JOURNAL]["entries"].clone())}).unwrap();
        let prepared=prepare_with(&repo,Kind::ReviewFeedback,&args,||Ok(inventory(&args,&generation)),uuid).unwrap();let state=repo.state.lock().unwrap().clone();assert_eq!(entries(&state).unwrap().len(),RECENT_COMPLETED+3);for (index,old) in arr(&original).iter().enumerate(){let mut expected=old.clone();expected["ticket"]=json!((index+1).to_string());assert_eq!(state[JOURNAL]["entries"][index],expected);}let done=submit_with(&repo,&prepared,||Ok(inventory(&args,&generation)),|_,_|Ok(()),||Ok(())).unwrap();assert_eq!(done["finalized"],true);assert_eq!(entries(&repo.state.lock().unwrap()).unwrap().len(),RECENT_COMPLETED);
    }
    #[test]
    fn dispatch_v2_legacy_bounds_are_not_relaxed_by_the_migration_allowance(){
        let (_temp,repo,_args,_generation,_old)=legacy_fixture();let original=repo.state.lock().unwrap().clone();let mut excessive=original.clone();excessive[JOURNAL]["largeNs"]=json!("x".repeat(4*1024*1024));let before=excessive.clone();assert!(migrate(&mut excessive).is_err());assert_eq!(excessive,before);let mut excessive=original;let entry=excessive[JOURNAL]["entries"][0].clone();excessive[JOURNAL]["entries"]=json!(vec![entry;LEGACY_MAX_ENTRIES+1]);let before=excessive.clone();assert!(migrate(&mut excessive).is_err());assert_eq!(excessive,before);
    }

    fn backup_path(repo:&Repository)->PathBuf{repo.data.with_extension("session-dispatch-v1-recovery.json")}
    #[test]
    fn dispatch_v2_recovery_backup_is_exact_private_and_retry_safe_after_canonical_failure(){
        use std::os::unix::fs::PermissionsExt;
        let (temp,repo,_args,_generation,_old)=legacy_fixture();let original=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();let path=backup_path(&repo);std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();assert!(ensure_migrated(&repo).is_err());assert_eq!(*repo.state.lock().unwrap(),original);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);assert_eq!(std::fs::read(&path).unwrap(),serde_json::to_vec(&original[JOURNAL]).unwrap());assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode()&0o777,0o600);assert!(!path.with_extension("tmp").exists());std::fs::remove_dir(repo.data.with_extension("tmp")).unwrap();drop(repo);
        let repo=Repository::open(temp.path().into(),None,true).unwrap();ensure_migrated(&repo).unwrap();assert_eq!(repo.state.lock().unwrap()[JOURNAL]["version"],2);assert_eq!(serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap(),original[JOURNAL]);let before=std::fs::read(&path).unwrap();ensure_migrated(&repo).unwrap();assert_eq!(std::fs::read(&path).unwrap(),before);assert!(repo.snapshot(true).get(JOURNAL).is_none());
    }
    #[test]
    fn dispatch_v2_recovery_backup_failures_preserve_both_canonical_and_existing_artifact(){
        use std::os::unix::fs::{PermissionsExt,symlink};
        for failure in ["conflict","symlink","directory","non-private"]{let (_temp,repo,_args,_generation,_old)=legacy_fixture();let path=backup_path(&repo);let target=path.with_extension("symlink-target");match failure{"directory"=>std::fs::create_dir(&path).unwrap(),"symlink"=>{std::fs::write(&target,b"unchanged symlink target").unwrap();symlink(&target,&path).unwrap();},_=>{let content=if failure=="conflict"{b"different legacy backup".to_vec()}else{serde_json::to_vec(&repo.state.lock().unwrap()[JOURNAL]).unwrap()};std::fs::write(&path,content).unwrap();std::fs::set_permissions(&path,std::fs::Permissions::from_mode(if failure=="non-private"{0o644}else{0o600})).unwrap();}}
            let before=repo.state.lock().unwrap().clone();let canonical=std::fs::read(&repo.data).unwrap();let old_backup=std::fs::read(&path).ok();assert!(ensure_migrated(&repo).is_err(),"{failure}");assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),canonical);assert_eq!(std::fs::read(&path).ok(),old_backup);if failure=="symlink"{assert_eq!(std::fs::read(&target).unwrap(),b"unchanged symlink target");}assert!(!path.with_extension("tmp").exists());
        }
    }
    #[test]
    fn dispatch_v2_recovery_backup_includes_explicit_empty_legacy_extensions_but_skips_absent_and_v2(){
        let (_temp,repo,_args,_generation)=fixture();ensure_migrated(&repo).unwrap();let path=backup_path(&repo);assert!(!path.exists());ensure_migrated(&repo).unwrap();assert!(!path.exists());repo.transaction(|s|{s[JOURNAL]=json!({"version":1,"entries":[],"extension":{"keep":"empty journal metadata"}});Ok(())}).unwrap();let expected=repo.state.lock().unwrap()[JOURNAL].clone();ensure_migrated(&repo).unwrap();assert_eq!(serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap(),expected);
    }

    #[test]
    fn dispatch_v2_stale_recovery_temporary_artifacts_do_not_block_restart_migration(){
        for directory in [false,true]{let (temp,repo,_args,_generation,_old)=legacy_fixture();let expected=repo.state.lock().unwrap()[JOURNAL].clone();let path=backup_path(&repo);let stale=path.with_extension("tmp");let stale_uuid=path.with_extension(format!("tmp-{}",uuid()));if directory{std::fs::create_dir(&stale).unwrap();std::fs::create_dir(&stale_uuid).unwrap();}else{std::fs::write(&stale,b"incomplete interrupted backup").unwrap();std::fs::write(&stale_uuid,b"interrupted UUID attempt").unwrap();}drop(repo);
            let repo=Repository::open(temp.path().into(),None,true).unwrap();ensure_migrated(&repo).unwrap();assert_eq!(repo.state.lock().unwrap()[JOURNAL]["version"],2);assert_eq!(serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap(),expected);if directory{assert!(stale.is_dir());assert!(stale_uuid.is_dir());}else{assert_eq!(std::fs::read(&stale).unwrap(),b"incomplete interrupted backup");assert_eq!(std::fs::read(&stale_uuid).unwrap(),b"interrupted UUID attempt");}let prefix=path.file_stem().unwrap().to_string_lossy().to_string()+".tmp-";let attempts=std::fs::read_dir(path.parent().unwrap()).unwrap().filter_map(|entry|entry.ok()).filter(|entry|entry.file_name().to_string_lossy().starts_with(&prefix)).count();assert_eq!(attempts,1,"only the unrelated stale attempt may remain");
        }
    }

}
