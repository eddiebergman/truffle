use super::*;
#[path = "review_access.rs"]
mod access;
pub use access::{history, read, update_note};

fn position(state: &Value, id: &str) -> Result<usize> {
    sid(id)?;
    arr(&state["sessions"])
        .iter()
        .position(|r| r["id"] == id)
        .ok_or(Error::new("Session unavailable."))
}
pub(super) fn draft(record: &Value) -> Value {
    if record["reviewDraft"].is_object() {
        record["reviewDraft"].clone()
    } else {
        json!({"comments":[],"note":"","revision":0})
    }
}
fn valid_pass(pass: &str) -> bool {
    !pass.is_empty() && pass.len() <= 80 && pass.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}
pub(super) fn versions(repo: &Repository, id: &str) -> Result<Value> {
    let record={let state=repo.state.lock().map_err(|_|Error::io())?;state["sessions"][position(&state,id)?].clone()};
    let roots=std::iter::once(text(&record,"cwd").to_owned()).chain(arr(&record["worktrees"]).iter().map(|t|text(t,"path").to_owned()).collect::<Vec<_>>()).filter(|r|!r.is_empty()).collect::<std::collections::BTreeSet<_>>();
    let mut result=json!({});
    for root in roots {
        match super::git_diff::review_version(repo,id,&root) {
            Ok(v)=>result[text(&v,"root")]=v["version"].clone(),
            Err(_)=>result[&root]=Value::Null,
        }
    }
    Ok(result)
}
fn freshness(before: &Value, current: &Value) -> &'static str {
    let Some(before)=before.as_object().filter(|m|!m.is_empty()) else {return "unverified"};
    if before.iter().any(|(root,value)| !value.is_null() && !current[root].is_null() && value!=&current[root]) || current.as_object().unwrap().keys().any(|root|!before.contains_key(root)) {return "outdated";}
    if before.iter().any(|(root,value)|value.is_null()||current[root].is_null()){return "unverified";}
    "current"
}
fn check(repo: &Repository, id: &str, mut review: Value) -> Result<Value> {
    let current=versions(repo,id)?;
    let mut assessments=arr(&review["assessments"]);
    for assessment in &mut assessments {assessment["freshness"]=json!(freshness(&assessment["versions"],&current));}
    review["assessments"]=json!(assessments);
    let mut comments=arr(&review["comments"]);
    for comment in &mut comments {
        let stamp=super::git_diff::review_file_stamp(text(comment,"root"),text(comment,"path"));
        comment["freshness"]=json!(match (comment["fileStamp"].as_str(),stamp.as_deref()) { (Some(a),Some(b)) if a==b=>"current",(Some(_),Some(_))=>"outdated",_=>"unverified" });
    }
    review["comments"]=json!(comments);
    review["freshness"]=json!(freshness(&review["run"]["versions"],&current));
    review["freshnessCheckedAt"]=json!(now());
    Ok(json!({"id":id,"review":review}))
}
fn summary_identity(repo:&Repository,value:&Value,max:usize)->Result<Value>{let text=value.as_str().filter(|value|!value.is_empty()&&value.len()<=max&&!value.chars().any(char::is_control)).ok_or(Error::new("Review identity is unavailable."))?;let mut safe=text.to_owned();super::commands::redact_text(repo,&mut safe);if safe!=text{return Err(Error::new("Review identity cannot be safely projected."));}Ok(json!(text))}
fn summary_time(value:&Value)->Value{value.as_f64().filter(|value|value.is_finite()&&*value>=0.0).map_or(Value::Null,|value|json!(value))}
fn summary_freshness(value:&Value)->Result<Value>{match value.as_str(){Some("current"|"outdated"|"unverified")=>Ok(value.clone()),_=>Err(Error::new("Review freshness is unavailable."))}}
/// Freshness is a local observation, not completion or delivery acknowledgement.
pub fn check_summary(repo:&Repository,args:&Value)->Result<Value>{
 let object=args.as_object().ok_or(Error::new("Review check arguments must be an object."))?;if object.len()!=1||!object.contains_key("id"){return Err(Error::new("Review check accepts only a saved chat ID."));}let id=sid(args["id"].as_str().ok_or(Error::new("Supply the saved chat ID."))?)?;summary_identity(repo,&json!(id),80)?;
 {let state=repo.state.lock().map_err(|_|Error::io())?;let review=draft(&state["sessions"][position(&state,&id)?]);if arr(&review["comments"]).len()>100||arr(&review["assessments"]).len()>100{return Err(Error::new("Review freshness summaries support up to 100 comments and 100 assessments."));}}
 let checked=action(repo,&json!({"id":id,"action":"check"}))?;let review=&checked["review"];
 let revision=review["revision"].as_u64().filter(|value|*value<=9_007_199_254_740_991).ok_or(Error::new("Review revision is unavailable."))?;
 let mut comments=vec![];let mut assessments=vec![];
 for (field,result)in [("comments",&mut comments),("assessments",&mut assessments)]{let rows=arr(&review[field]);if rows.len()>100{return Err(Error::new("Review freshness summary exceeds its row limit."));}for row in rows{let mut entry=json!({"id":summary_identity(repo,&row["id"],120)?,"freshness":summary_freshness(&row["freshness"])?,"reviewedAt":summary_time(&row["reviewedAt"])});if field=="assessments"{entry["sentAt"]=summary_time(&row["sentAt"]);}result.push(entry);}}
 let run=if review["run"].is_object(){let raw=&review["run"];let state=raw["state"].as_str().filter(|value|["requested","reviewing","complete","failed","cancelled"].contains(value)).ok_or(Error::new("Review run state is unavailable."))?;json!({"id":summary_identity(repo,&raw["id"],80)?,"state":state,"scope":raw["scope"].as_str().filter(|value|*value=="conversation"),"startedAt":summary_time(&raw["startedAt"]),"updatedAt":summary_time(&raw["updatedAt"]),"completedAt":summary_time(&raw["completedAt"])})}else{Value::Null};
 let result=json!({"id":id,"revision":revision,"run":run,"freshness":summary_freshness(&review["freshness"])?,"freshnessCheckedAt":summary_time(&review["freshnessCheckedAt"]),"comments":comments,"assessments":assessments});
 // Fixed status tags are actionable too; do not let generic redaction silently
 // turn a valid freshness/run state into a different result after this owner.
 for value in ["current","outdated","unverified","requested","reviewing","complete","failed","cancelled","conversation"]{summary_identity(repo,&json!(value),80)?;}
 Ok(result)
}
fn set_request(review:&mut Value,request:&str,baseline:&Value)->Result<()> {
    let request=sid(request)?;
    if review["run"]["id"]!=request {
        if ["requested","reviewing"].contains(&text(&review["run"],"state")){return Err(Error::new("A review is already active. Clear its indicator before starting another."));}
        review["run"]=json!({"id":request,"state":"requested","scope":"conversation","versions":baseline,"startedAt":now(),"updatedAt":now(),"passes":{},"inputCommentIds":arr(&review["comments"]).iter().map(|c|c["id"].clone()).collect::<Vec<_>>(),"inputNote":review["note"]});
    }
    Ok(())
}
fn remove_or_ack(review:&mut Value,saved:&mut Vec<Value>,p:&Value,operation:&str){
                let ids = arr(&p["commentIds"]);
                saved.retain(|c| !ids.contains(&c["id"]));
                if operation == "remove" && p["assessmentIds"].is_array() {
                    let ids=arr(&p["assessmentIds"]);
                    let mut assessments=arr(&review["assessments"]);
                    assessments.retain(|a| !ids.contains(&a["id"]));
                    review["assessments"]=json!(assessments);
                }
                if operation == "ack" {
                    let ids=arr(&p["assessmentIds"]);
                    let mut assessments=arr(&review["assessments"]);
                    for assessment in &mut assessments {
                        if ids.contains(&assessment["id"]) && assessment.get("sentAt").is_none() { assessment["sentAt"]=json!(now()); }
                    }
                    if !assessments.is_empty() { review["assessments"]=json!(assessments); }
                }
                if operation == "ack" && p["note"] == review["note"] { review["note"] = json!(""); }
}
pub(super) fn reserve_request(state:&mut Value,id:&str,request:&str,revision:u64,baseline:&Value)->Result<Value>{
    let index=position(state,id)?;let mut review=draft(&state["sessions"][index]);
    if review["revision"]!=revision{return Err(Error::new("The review changed. Reload it before requesting a review."));}
    let before=review.clone();set_request(&mut review,request,baseline)?;
    if review!=before{review["revision"]=json!(revision+1);}
    state["sessions"][index]["reviewDraft"]=review.clone();Ok(review)
}
pub(super) fn acknowledge_dispatch(state:&mut Value,id:&str,ack:&Value)->Result<()> {
    let index=position(state,id)?;
    if ack["historyId"]!=state["sessions"][index]["reviewHistory"]["entries"][0]["id"]{return Ok(());}
    let mut review=draft(&state["sessions"][index]);let before=review.clone();let mut saved=arr(&review["comments"]);
    let mut exact=ack.clone();
    exact["commentIds"]=json!(arr(&ack["comments"]).iter().filter(|captured|saved.iter().any(|current|current==*captured)).map(|captured|captured["id"].clone()).collect::<Vec<_>>());
    exact["assessmentIds"]=json!(arr(&ack["assessments"]).iter().filter(|captured|arr(&review["assessments"]).iter().any(|current|current==*captured)).map(|captured|captured["id"].clone()).collect::<Vec<_>>());
    remove_or_ack(&mut review,&mut saved,&exact,"ack");review["comments"]=json!(saved);
    if review!=before{review["revision"]=json!(before["revision"].as_u64().unwrap_or(0)+1);}
    state["sessions"][index]["reviewDraft"]=review;super::guidance::complete(state,"review.pending-comments");Ok(())
}
pub(super) fn cancel_dispatch(state:&mut Value,id:&str,operation:&str)->Result<()> {
    let index=position(state,id)?;let mut review=draft(&state["sessions"][index]);
    if review["run"]["id"]==operation&&["requested","reviewing"].contains(&text(&review["run"],"state")){review["run"]["state"]=json!("cancelled");review["run"]["updatedAt"]=json!(now());review["revision"]=json!(review["revision"].as_u64().unwrap_or(0)+1);state["sessions"][index]["reviewDraft"]=review;}
    Ok(())
}
pub fn action(repo: &Repository, p: &Value) -> Result<Value> {
    let id = text(p, "id");
    let operation = text(p, "action");
    if operation == "history" {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        let history = &state["sessions"][position(&state,id)?]["reviewHistory"];
        if !history.is_null() && history["version"] != 1 { return Err(Error::new("Unsupported review history version.")); }
        return Ok(json!({"id":id,"reviews":arr(&history["entries"]),"limit":20}));
    }
    if operation == "read" || operation == "check" {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        let review=draft(&state["sessions"][position(&state,id)?]);drop(state);
        return if operation=="check" {check(repo,id,review)} else {Ok(json!({"id":id,"review":review}))};
    }
    let baseline=if operation=="request" || operation=="status" && p["state"]=="reviewing" && text(p,"pass").is_empty() {Some(versions(repo,id)?)}else{None};
    let mut comments = arr(&p["comments"]);
    if operation == "append" {
        if (comments.is_empty() && !p["overall"].is_object()) || comments.len() > 100 || serde_json::to_vec(p)?.len() > 96 * 1024 {
            return Err(Error::new("Supply comments or an overall assessment within 96 KiB."));
        }
        let mut files = std::collections::HashMap::new();
        for comment in &mut comments {
            let key = (
                text(comment, "root").to_owned(),
                text(comment, "path").to_owned(),
            );
            let file = if let Some(file) = files.get(&key) {
                file
            } else {
                let file = super::git_diff::file_for_session(repo, id, &key.1, &key.0)?;
                files.entry(key).or_insert(file)
            };
            let side = text(comment, "side");
            let content = match side {
                "additions" => text(&file, "newContent"),
                "deletions" => text(&file, "oldContent"),
                _ => return Err(Error::new("Comment side must be additions or deletions.")),
            };
            let start = comment["start"].as_u64().unwrap_or(0);
            let end = comment["end"].as_u64().unwrap_or(start);
            if start == 0
                || end < start
                || end as usize > content.lines().count()
                || text(comment, "text").trim().is_empty()
                || text(comment, "text").len() > 4000
            {
                return Err(Error::new(
                    "Select valid lines and supply a comment of at most 4000 bytes.",
                ));
            }
            if !text(comment, "fingerprint").is_empty()
                && comment["fingerprint"] != file["fingerprint"]
            {
                return Err(Error::new(
                    "The file changed. Reload the diff before adding this comment.",
                ));
            }
            let key = text(comment, "id").to_owned();
            if key.len() > 120
                || !key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            {
                return Err(Error::new("Invalid comment ID."));
            }
            let author = text(comment, "author").trim();
            if author.len() > 120 {
                return Err(Error::new("Comment author exceeds 120 bytes."));
            }
            let stamp=super::git_diff::review_file_stamp(text(&file,"root"),text(&file,"path"));
            *comment = json!({"id":if key.is_empty(){uuid()}else{key},"author":if author.is_empty(){"You"}else{author},"createdAt":now(),"fileStamp":stamp,"requestId":p["requestId"],"path":file["path"],"root":file["root"],"side":side,"start":start,"end":end,"text":text(comment,"text").trim(),"baseCommit":file["baseCommit"],"fingerprint":file["fingerprint"],"excerpt":content.split('\n').skip(start as usize-1).take((end-start+1).min(20) as usize).collect::<Vec<_>>().join("\n")});
        }
    }
    repo.transaction(|state| {
        let index = position(state,id)?;
        let mut review = draft(&state["sessions"][index]);
        let before = review.clone();
        let mut saved = arr(&review["comments"]);
        if operation == "status" || p.get("requestId").is_some() && operation == "append" {
            if text(p,"requestId").is_empty() || review["run"]["id"] != p["requestId"] {
                return Err(Error::new("This review request is no longer current. Read its draft before updating it."));
            }
            if !["requested","reviewing"].contains(&text(&review["run"],"state")) {
                return Err(Error::new("This review request has already finished."));
            }
        }
        if operation == "append" && p.get("overall").is_some() {
            let pass = text(p,"pass");
            let overall = &p["overall"];
            if !valid_pass(pass)
                || text(p,"requestId").is_empty()
                || text(overall,"author").trim().is_empty()
                || text(overall,"author").len() > 120
                || text(overall,"text").trim().is_empty()
                || text(overall,"text").len() > 8000 {
                return Err(Error::new("An overall assessment needs a current request, review pass, author and text of at most 8000 bytes."));
            }
            let key = format!("{}-{pass}",text(p,"requestId"));
            let mut assessments=arr(&review["assessments"]);
            if let Some(existing)=assessments.iter().find(|a| a["id"] == key) {
                if text(existing,"author") != text(overall,"author").trim() || text(existing,"text") != text(overall,"text").trim() {
                    return Err(Error::new("This reviewer already submitted a different assessment."));
                }
            } else {
                assessments.push(json!({"id":key,"requestId":p["requestId"],"pass":pass,"author":text(overall,"author").trim(),"text":text(overall,"text").trim(),"root":review["run"]["root"],"baseCommit":review["run"]["baseCommit"],"versions":review["run"]["versions"],"createdAt":now()}));
            }
            review["assessments"]=json!(assessments);
        }
        match operation {
            "new" => {
                if p["revision"].as_u64().is_none() || p["revision"] != review["revision"] {
                    return Err(Error::new("The review changed. Reload it before starting a new review."));
                }
                if ["requested","reviewing"].contains(&text(&review["run"],"state")) {
                    return Err(Error::new("Finish or cancel the active review before starting a new review."));
                }
                let history = &state["sessions"][index]["reviewHistory"];
                if !history.is_null() && history["version"] != 1 { return Err(Error::new("Unsupported review history version.")); }
                let mut entries = arr(&history["entries"]);
                if !saved.is_empty() || !text(&review,"note").is_empty() || !arr(&review["assessments"]).is_empty() || review["run"].is_object() {
                    entries.insert(0,json!({"id":uuid(),"archivedAt":now(),"review":review}));
                }
                entries.truncate(20);
                while serde_json::to_vec(&entries)?.len() > 1024*1024 { entries.pop(); }
                state["sessions"][index]["reviewHistory"] = json!({"version":1,"entries":entries});
                review = json!({"comments":[],"note":"","revision":before["revision"]});
                saved.clear();
            }
            "request" => { set_request(&mut review,text(p,"requestId"),baseline.as_ref().unwrap())?; }
            "status" => {if review["run"]["state"]=="requested" {if let Some(versions)=&baseline{review["run"]["versions"]=versions.clone();}}}

            "append" => {
                for comment in comments {
                    if let Some(existing) = saved.iter().find(|c| c["id"] == comment["id"]) {
                        for field in ["author","path","root","side","start","end","text","fingerprint"] {
                            if existing[field] != comment[field] { return Err(Error::new("Comment ID already exists with different content.")); }
                        }
                    } else { saved.push(comment); }
                }
                if saved.len() > 100 { return Err(Error::new("Send or remove comments before adding more. A draft holds 100 comments.")); }
            }
            "remove" | "ack" => {remove_or_ack(&mut review,&mut saved,p,operation);}
            "note" => {
                let note = p["note"].as_str().ok_or(Error::new("Expected review note."))?;
                if note.len() > 8000 { return Err(Error::new("Review note exceeds 8000 bytes.")); }
                if p["previousNote"] != review["note"] { return Err(Error::new("The review note changed. Reload it before editing.")); }
                review["note"] = json!(note);
            }
            _ => return Err(Error::new("Unknown review action.")),
        }
        if operation == "status" || operation == "append" && p.get("requestId").is_some() {
            let status=text(p,"state");
            let pass=text(p,"pass");
            if !status.is_empty() && (if pass.is_empty(){text(&review["run"],"state")!=status}else{text(&review["run"]["passes"],pass)!=status}) {
                if !["reviewing","complete","failed","cancelled"].contains(&status) { return Err(Error::new("Invalid review state.")); }
                if pass.is_empty() {
                    if status=="complete" {
                        let receipt=p.get("reviewedRevision");
                        if receipt.is_some()&&receipt!=review.get("revision"){return Err(Error::new("Review feedback changed. Read the draft again before completing this round."));}
                        let inputs=arr(&review["run"]["inputCommentIds"]);
                        for comment in &mut saved {if receipt.is_some()||inputs.contains(&comment["id"])||comment["requestId"]==review["run"]["id"]{comment["reviewedAt"]=json!(now());}}
                        let mut assessments=arr(&review["assessments"]);for assessment in &mut assessments{if receipt.is_some()||assessment["requestId"]==review["run"]["id"]{assessment["reviewedAt"]=json!(now());}}review["assessments"]=json!(assessments);
                        if receipt.is_some()||review["note"]==review["run"]["inputNote"]{review["reviewedNote"]=review["note"].clone();}
                        review["run"]["completedAt"]=json!(now());
                    }
                    review["run"]["state"]=json!(status);
                }
                else {
                    if !valid_pass(pass) { return Err(Error::new("Invalid review pass.")); }
                    review["run"]["passes"][pass]=json!(status);
                    review["run"]["state"]=json!("reviewing");
                }
                review["run"]["updatedAt"]=json!(now());
            }
        }
        review["comments"] = json!(saved);
        if serde_json::to_vec(&review)?.len() > 256*1024 { return Err(Error::new("Review draft exceeds 256 KiB.")); }
        if review != before { review["revision"] = json!(before["revision"].as_u64().unwrap_or(0)+1); }
        state["sessions"][index]["reviewDraft"] = review.clone();
        if operation == "append" && arr(&review["comments"]).iter().any(|c| c["author"] == "You") { super::guidance::complete(state,"review.comment-selection"); }
        if operation == "ack" { super::guidance::complete(state,"review.pending-comments"); }
        if operation == "status" && ["reviewing","complete"].contains(&text(p,"state")) { super::guidance::complete(state,"review.agent-scope"); super::guidance::complete(state,"review.pending-comments"); }
        Ok(json!({"id":id,"review":review}))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap();
            assert!(status.status.success());
        };
        git(&["init", "-b", "main"]);
        std::fs::write(root.join("code.txt"), "old\n").unwrap();
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
        std::fs::write(root.join("code.txt"), "new\nmore\n").unwrap();
        let repo = Repository::open(temp.path().to_path_buf(), None, true).unwrap();
        let record = repo
            .transaction(|s| {
                workflows::save_session(
                    s,
                    &json!({"agent":"claude","cwd":root,"title":"Review fixture","itemIds":[]}),
                )
            })
            .unwrap();
        (temp, repo, text(&record, "id").to_owned(), root)
    }
    fn append(repo: &Repository, id: &str, key: &str, author: &str) -> Value {
        action(repo,&json!({"id":id,"action":"append","comments":[{"id":key,"author":author,"path":"code.txt","side":"additions","start":1,"end":2,"text":"Check this"}]})).unwrap()
    }
    #[test]
    fn new_review_preserves_history_and_rejects_stale_or_active_resets() {
        let (_tmp,repo,id,_root)=fixture();
        let first=append(&repo,&id,"one","You");
        assert!(action(&repo,&json!({"id":id,"action":"new","revision":0})).is_err());
        let request=uuid();
        let active=action(&repo,&json!({"id":id,"action":"request","requestId":request})).unwrap();
        assert!(action(&repo,&json!({"id":id,"action":"new","revision":active["review"]["revision"]})).is_err());
        let done=action(&repo,&json!({"id":id,"action":"status","requestId":request,"state":"complete"})).unwrap();
        let fresh=action(&repo,&json!({"id":id,"action":"new","revision":done["review"]["revision"]})).unwrap();
        assert_eq!(fresh["review"]["comments"],json!([]));
        assert!(fresh["review"]["run"].is_null());
        assert_eq!(fresh["review"]["revision"].as_u64().unwrap(),done["review"]["revision"].as_u64().unwrap()+1);
        let history=action(&repo,&json!({"id":id,"action":"history"})).unwrap();
        assert_eq!(history["reviews"][0]["review"],done["review"]);
        assert_eq!(history["reviews"][0]["review"]["comments"][0]["text"],first["review"]["comments"][0]["text"]);
        assert!(action(&repo,&json!({"id":id,"action":"status","requestId":request,"state":"complete"})).is_err());
        let retained=action(&repo,&json!({"id":id,"action":"history"})).unwrap();
        assert_eq!(retained,history);
        for n in 0..22 {
            let note=action(&repo,&json!({"id":id,"action":"note","previousNote":"","note":format!("Round {n}")})).unwrap();
            action(&repo,&json!({"id":id,"action":"new","revision":note["review"]["revision"]})).unwrap();
        }
        let history=action(&repo,&json!({"id":id,"action":"history"})).unwrap();
        assert_eq!(arr(&history["reviews"]).len(),20);
        assert_eq!(history["reviews"][0]["review"]["note"],"Round 21");
    }
    #[test]
    fn multiple_sources_append_idempotently_and_ack_preserves_later_comments() {
        let (tmp, repo, id, root) = fixture();
        let alias = tmp.path().join("alias");
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        repo.transaction(|s| {
            s["sessions"][0]["cwd"] = json!(alias);
            Ok(())
        })
        .unwrap();
        let first = append(&repo, &id, "one", "You");
        assert_eq!(arr(&first["review"]["comments"]).len(), 1);
        let again = append(&repo, &id, "one", "You");
        assert_eq!(again, first);
        append(&repo, &id, "two", "Background agent");
        let result = action(
            &repo,
            &json!({"id":id,"action":"ack","commentIds":["one"],"note":""}),
        )
        .unwrap();
        assert_eq!(arr(&result["review"]["comments"]).len(), 1);
        assert_eq!(
            result["review"]["comments"][0]["author"],
            "Background agent"
        );
        let reply = workflows::registration_reply(&result);
        assert_eq!(reply["review"], result["review"]);
    }
    #[test]
    fn bad_batch_rolls_back_and_note_edit_is_guarded() {
        let (_tmp, repo, id, root) = fixture();
        let first = append(&repo, &id, "one", "You");
        let bad = json!({"id":id,"action":"append","comments":[{"id":"two","path":"code.txt","side":"additions","start":1,"text":"Valid"},{"path":"../outside","side":"additions","start":1,"text":"Invalid"}]});
        assert!(action(&repo, &bad).is_err());
        assert_eq!(
            action(&repo, &json!({"id":id,"action":"read"})).unwrap(),
            first
        );
        assert!(action(&repo,&json!({"id":id,"action":"append","comments":[{"path":"code.txt","root":root,"side":"additions","start":1,"fingerprint":"outdated","text":"Check"}]})).is_err());
        action(
            &repo,
            &json!({"id":id,"action":"note","note":"first","previousNote":""}),
        )
        .unwrap();
        assert!(action(
            &repo,
            &json!({"id":id,"action":"note","note":"stale edit","previousNote":""})
        )
        .is_err());
        action(
            &repo,
            &json!({"id":id,"action":"ack","commentIds":[],"note":"old"}),
        )
        .unwrap();
        assert_eq!(
            action(&repo, &json!({"id":id,"action":"read"})).unwrap()["review"]["note"],
            "first"
        );
    }
    #[test]
    fn reviewer_assessments_are_scoped_durable_and_ack_preserves_new_arrivals() {
        let (_tmp,repo,id,root)=fixture();
        let request=uuid();
        action(&repo,&json!({"id":id,"action":"request","requestId":request,"root":root})).unwrap();
        action(&repo,&json!({"id":id,"action":"note","note":"My summary","previousNote":""})).unwrap();
        let report=json!({"id":id,"operation":"review","requestId":request,"pass":"simplification","state":"complete","report":{"comments":[],"overall":{"author":"Agent A","text":"No simplification needed. Scope: code.txt."}}});
        let first=workflows::register(&repo,&report).unwrap();
        assert_eq!(first["review"]["assessments"][0]["author"],"Agent A");
        assert_eq!(workflows::register(&repo,&report).unwrap(),first);
        let mut invalid=report.clone();invalid["requestId"]=json!(uuid());
        assert!(workflows::register(&repo,&invalid).is_err());
        invalid=report.clone();invalid["report"]["overall"]["text"]=json!("Changed");
        assert!(workflows::register(&repo,&invalid).is_err());
        invalid=report.clone();invalid["pass"]=json!("correctness");invalid["report"]["overall"]["text"]=json!("x".repeat(8001));
        assert!(workflows::register(&repo,&invalid).is_err());
        let mut second=report.clone();second["pass"]=json!("performance");second["report"]["overall"]["author"]=json!("Agent B");
        workflows::register(&repo,&second).unwrap();
        let result=action(&repo,&json!({"id":id,"action":"ack","commentIds":[],"assessmentIds":[first["review"]["assessments"][0]["id"]],"note":"old summary"})).unwrap();
        assert_eq!(arr(&result["review"]["assessments"]).len(),2);
        assert!(result["review"]["assessments"][0]["sentAt"].is_number());
        assert!(result["review"]["assessments"][1]["sentAt"].is_null());
        assert_eq!(result["review"]["note"],"My summary");
        assert_eq!(workflows::registration_reply(&result)["review"],result["review"]);
    }
    #[test]
    fn completion_consumes_only_verified_feedback_and_keeps_later_comments_pending(){
        let (_tmp,repo,id,_root)=fixture();let request=uuid();append(&repo,&id,"before","You");
        let previous=uuid();action(&repo,&json!({"id":id,"action":"request","requestId":previous})).unwrap();
        action(&repo,&json!({"id":id,"action":"append","requestId":previous,"pass":"correctness","overall":{"author":"Claude","text":"Earlier review"}})).unwrap();
        action(&repo,&json!({"id":id,"action":"status","requestId":previous,"state":"cancelled"})).unwrap();
        action(&repo,&json!({"id":id,"action":"request","requestId":request})).unwrap();
        append(&repo,&id,"during","You");let read=action(&repo,&json!({"id":id,"action":"read"})).unwrap();let revision=read["review"]["revision"].clone();
        append(&repo,&id,"late","You");
        assert!(action(&repo,&json!({"id":id,"action":"status","requestId":request,"state":"complete","reviewedRevision":revision})).is_err());
        let current=action(&repo,&json!({"id":id,"action":"read"})).unwrap();
        let done=action(&repo,&json!({"id":id,"action":"status","requestId":request,"state":"complete","reviewedRevision":current["review"]["revision"]})).unwrap();
        assert_eq!(arr(&done["review"]["comments"]).len(),3);assert!(arr(&done["review"]["comments"]).iter().all(|c|c["reviewedAt"].is_number()));
        assert!(done["review"]["assessments"][0]["reviewedAt"].is_number());
        let after=append(&repo,&id,"after","You");assert!(after["review"]["comments"][3]["reviewedAt"].is_null());
    }
    #[test]
    fn conversation_review_accepts_new_roots_and_marks_changed_versions_without_writing() {
        let (tmp,repo,id,root)=fixture();let other=tmp.path().join("other");
        assert!(std::process::Command::new("git").args(["clone","--quiet"]).arg(&root).arg(&other).status().unwrap().success());
        std::fs::write(other.join("code.txt"),"review this second repository\n").unwrap();
        let request=uuid();
        action(&repo,&json!({"id":id,"action":"request","requestId":request})).unwrap();
        repo.transaction(|s|{s["sessions"][0]["worktrees"]=json!([{"path":other}]);Ok(())}).unwrap();
        action(&repo,&json!({"id":id,"action":"status","requestId":request,"state":"reviewing"})).unwrap();
        action(&repo,&json!({"id":id,"action":"append","requestId":request,"pass":"cross-repo","overall":{"author":"Agent","text":"Checked both repositories."},"comments":[{"id":"other-comment","root":other,"path":"code.txt","side":"additions","start":1,"end":1,"text":"Check the other repository."}]})).unwrap();
        let revision=repo.snapshot(false)["revision"].clone();
        let checked=action(&repo,&json!({"id":id,"action":"check"})).unwrap();
        assert_eq!(checked["review"]["freshness"],"current");assert_eq!(checked["review"]["comments"][0]["freshness"],"current");
        std::fs::write(other.join("code.txt"),"changed in second repository\n").unwrap();
        let checked=action(&repo,&json!({"id":id,"action":"check"})).unwrap();
        assert_eq!(checked["review"]["freshness"],"outdated");assert_eq!(checked["review"]["assessments"][0]["freshness"],"outdated");assert_eq!(checked["review"]["comments"][0]["freshness"],"outdated");
        assert_eq!(repo.snapshot(false)["revision"],revision);
        assert_eq!(freshness(&json!({"/missing":"before"}),&json!({"/missing":null})),"unverified");
    }
    #[test]
    fn review_progress_accepts_dynamic_reviewers_and_parent_completion() {
        let (_tmp, repo, id, root) = fixture();
        let request = uuid();
        assert!(workflows::register(&repo,&json!({"id":id,"operation":"review","report":{"comments":"invalid"}})).is_err());
        let start = action(
            &repo,
            &json!({"id":id,"action":"request","requestId":request,"root":root}),
        )
        .unwrap();
        assert_eq!(start["review"]["run"]["state"], "requested");
        assert!(action(
            &repo,
            &json!({"id":id,"action":"request","requestId":uuid(),"root":root})
        )
        .is_err());
        assert!(action(
            &repo,
            &json!({"id":id,"action":"status","requestId":uuid(),"state":"complete"})
        )
        .is_err());
        assert_eq!(start["review"]["run"]["passes"], json!({}));
        for pass in ["security", "api-contract"] {
            workflows::register(&repo,&json!({"id":id,"operation":"review","requestId":request,"pass":pass,"state":"reviewing"})).unwrap();
            workflows::register(&repo,&json!({"id":id,"operation":"review","requestId":request,"pass":pass,"state":"complete"})).unwrap();
        }
        workflows::register(&repo,&json!({"id":id,"operation":"review","requestId":request,"pass":"extra-reviewer","state":"reviewing"})).unwrap();
        assert!(workflows::register(&repo,&json!({"id":id,"operation":"review","requestId":request,"pass":"invalid name","state":"reviewing"})).is_err());
        let end = workflows::register(
            &repo,
            &json!({"id":id,"operation":"review","requestId":request,"state":"complete"}),
        )
        .unwrap();
        assert_eq!(end["review"]["run"]["state"], "complete");
        assert!(action(
            &repo,
            &json!({"id":id,"action":"status","requestId":request,"state":"reviewing"})
        )
        .is_err());
        assert!(action(
            &repo,
            &json!({"id":id,"action":"request","requestId":uuid(),"root":root})
        )
        .is_ok());
    }
    #[test]
    fn review_check_command_summary_is_read_only_fresh_and_excludes_private_draft_payloads(){
        let (_temp,repo,id,root)=fixture();append(&repo,&id,"summary-comment","You");let request=uuid();action(&repo,&json!({"id":id,"action":"request","requestId":request})).unwrap();action(&repo,&json!({"id":id,"action":"append","requestId":request,"pass":"correctness","overall":{"author":"Agent","text":"Private assessment text"}})).unwrap();repo.transaction(|s|{let review=&mut s["sessions"][0]["reviewDraft"];review["privateNs"]=json!("private-summary-canary");review["note"]=json!("private-summary-canary");review["run"]["privateNs"]=json!("private-summary-canary");review["comments"][0]["privateNs"]=json!("private-summary-canary");review["comments"][0]["reviewedAt"]=json!(10);review["assessments"][0]["sentAt"]=json!(20);review["assessments"][0]["privateNs"]=json!("private-summary-canary");Ok(())}).unwrap();let before=repo.state.lock().unwrap().clone();let bytes=std::fs::read(&repo.data).unwrap();let result=super::super::commands::invoke(&repo,&json!({"name":"review.check","args":{"id":id}}),false).unwrap();assert_eq!(result["id"],id);assert_eq!(result["revision"],before["sessions"][0]["reviewDraft"]["revision"]);assert_eq!(result["freshness"],"current");assert_eq!(result["run"]["id"],request);assert_eq!(result["comments"][0],json!({"id":"summary-comment","freshness":"current","reviewedAt":10.0}));assert_eq!(result["assessments"][0]["sentAt"],20.0);for private in ["private-summary-canary","Private assessment text","Check this","versions","code.txt"]{assert!(!result.to_string().contains(private));}assert_eq!(*repo.state.lock().unwrap(),before);assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);
        std::fs::write(root.join("code.txt"),"Changed since review\n").unwrap();let changed=check_summary(&repo,&json!({"id":id})).unwrap();assert_eq!(changed["freshness"],"outdated");assert_eq!(changed["comments"][0]["freshness"],"outdated");assert_eq!(changed["assessments"][0]["freshness"],"outdated");std::fs::remove_dir_all(root).unwrap();assert_eq!(check_summary(&repo,&json!({"id":id})).unwrap()["freshness"],"unverified");assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);
    }
    #[test]
    fn review_check_summary_rejects_extra_arguments_and_excessive_status_rows(){let (_temp,repo,id,_)=fixture();for args in [json!({"id":id,"cwd":"/private"}),json!({"id":id,"action":"clear"}),json!({"id":7})]{assert!(check_summary(&repo,&args).is_err());}repo.transaction(|s|{s["sessions"][0]["reviewDraft"]=json!({"revision":0,"comments":[],"assessments":(0..101).map(|index|json!({"id":format!("assessment-{index}")})).collect::<Vec<_>>()});Ok(())}).unwrap();let bytes=std::fs::read(&repo.data).unwrap();assert!(check_summary(&repo,&json!({"id":id})).unwrap_err().message.contains("100"));assert_eq!(std::fs::read(&repo.data).unwrap(),bytes);}

}
