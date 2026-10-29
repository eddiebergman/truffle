use super::*;

pub fn normalized(value: &str) -> String {
    value.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

pub fn effective(state: &Value, item: &Value) -> Value {
    let mut row = item.clone();
    let overrides = &state["overrides"][text(item, "id")];
    row["sourceStatusCode"] = item["status"].clone();
    if !text(overrides, "status").is_empty() {
        row["status"] = overrides["status"].clone();
    }
    for field in ["pinned", "hidden"] { row[field] = json!(overrides[field] == true); }
    row["overridden"] = json!(!text(overrides, "status").is_empty());
    if item["source"] == "notes" { row["kind"] = json!("Project"); }
    row["snoozed"] = json!(domain::snoozed(state, item));
    row["actionReasons"] = json!(domain::reasons(&row));
    row
}

pub fn project(item: &Value, detail_limit: usize) -> Value {
    let mut row = json!({});
    for key in ["id", "source", "kind", "title", "reference", "url", "project", "projectId", "status", "sourceStatus", "sourceStatusCode", "ci", "createdAt", "updatedAt", "parent", "relations", "pinned", "hidden", "snoozed", "overridden", "archivedAt", "sourceUnavailable", "actionReasons"] {
        if let Some(value) = item.get(key) { row[key] = value.clone(); }
    }
    row["detail"] = json!(text(item, "detail").chars().take(detail_limit).collect::<String>());
    row
}

fn rank(item: &Value, words: &[String]) -> Option<(usize, bool, Vec<&'static str>)> {
    let fields = [("title", 40), ("reference", 35), ("project", 20), ("sourceStatus", 15), ("detail", 5)];
    let mut values = fields.iter().map(|(name, weight)| (*name, *weight, normalized(text(item, name)))).collect::<Vec<_>>();
    values.push(("relations",15,normalized(&arr(&item["relations"]).iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "))));
    let mut score = 0;
    let mut direct = true;
    let mut matched = vec![];
    for word in words {
        let mut best = None;
        for (name, weight, value) in &values {
            let found = if let Some(index) = value.find(word) {
                Some((100 + weight + usize::from(index == 0) * 10, true, *name))
            } else if *name=="detail" { None } else {
                let mut rest = value.as_str();
                let mut found = true;
                for ch in word.chars() {
                    if let Some(index) = rest.find(ch) { rest = &rest[index + ch.len_utf8()..]; }
                    else { found = false; break; }
                }
                found.then_some((*weight, false, *name))
            };
            if let Some(found) = found { if best.as_ref().is_none_or(|current: &(usize, bool, &str)| found.0 > current.0) { best = Some(found); } }
        }
        let (part, exact, name) = best?;
        score += part;
        direct &= exact;
        if !matched.contains(&name) { matched.push(name); }
    }
    Some((score, direct, matched))
}

pub fn search(repo: &Repository, args: &Value) -> Result<Value> {
    let view_id = checked(args,"viewId",80,false)?;
    let selected = if view_id.is_empty() { None } else {
        Some(super::views::load(repo)?.views.into_iter().find(|view|view.id==view_id).ok_or(Error::new("Saved view unavailable. Read views.read before searching."))?)
    };
    let query = if args.get("query").is_none() { selected.as_ref().map(|view|view.query.clone()).unwrap_or_default() } else { checked(args, "query", 500, false)? };
    let source = text(args, "source");
    if !["", "all", "linear", "github", "sentry", "notes"].contains(&source) { return Err(Error::new("Unknown work source.")); }
    let lifecycle = text(args, "lifecycle");
    if !["", "active", "completed", "any"].contains(&lifecycle) { return Err(Error::new("Choose active, completed, or any lifecycle.")); }
    for key in ["includeHidden", "includeArchived", "includeSnoozed"] {
        if args.get(key).is_some_and(|value| !value.is_boolean()) { return Err(Error::new(format!("{key} must be a boolean."))); }
    }
    let words = query.split_whitespace().map(normalized).filter(|word| !word.is_empty()).collect::<Vec<_>>();
    let state = repo.state.lock().map_err(|_| Error::io())?;
    let catalog = if args["includeArchived"] == true { domain::catalog(&state).into_values().collect::<Vec<_>>() }
        else { arr(&state["items"]).into_iter().chain(arr(&state["notes"])).collect() };
    let mut rows = vec![];
    let evaluation_time = now();
    for item in catalog {
        let row = effective(&state, &item);
        if selected.as_ref().is_some_and(|view| !super::views::evaluate_at(view,&row,evaluation_time)) { continue; }
        let archived = !row["archivedAt"].is_null() && row["archivedAt"] != "" && row["archivedAt"] != 0;
        if (!source.is_empty() && source != "all" && row["source"] != source)
            || (args["includeArchived"] != true && archived)
            || (args["includeHidden"] != true && row["hidden"] == true)
            || (args["includeSnoozed"] != true && row["snoozed"] == true)
            || (lifecycle == "active" && row["status"] == "done")
            || (lifecycle == "completed" && row["status"] != "done") { continue; }
        if let Some((score, direct, fields)) = rank(&row, &words) { rows.push((score, direct, fields, row)); }
    }
    if rows.iter().any(|row| row.1) { rows.retain(|row| row.1); }
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| text(&a.3, "title").cmp(text(&b.3, "title"))).then_with(|| text(&a.3, "id").cmp(text(&b.3, "id"))));
    let offset = args["offset"].as_u64().unwrap_or(0).min(100000) as usize;
    let total = rows.len();
    let rows = rows.into_iter().skip(offset).take(40).map(|(score, direct, fields, row)| {
        let mut row = project(&row, 2000); row["match"] = json!({"score":score,"kind":if direct {"direct"} else {"fuzzy"},"fields":fields}); row
    }).collect::<Vec<_>>();
    Ok(json!({"items":rows,"total":total,"offset":offset,"nextOffset":(offset+40 < total).then_some(offset+40),"cached":true,"sourceSyncTimes":state["sources"].as_object().map(|sources| sources.iter().map(|(key, value)| (key.clone(),value["lastSuccess"].clone())).collect::<serde_json::Map<_,_>>())}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn work_search_uses_effective_state_and_punctuation_and_keeps_exact_results_first() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        repo.transaction(|state| {
            state["items"] = json!([
                {"id":"one","source":"linear","title":"Thinking-v2 latency","reference":"ENG-1","status":"todo","detail":""},
                {"id":"two","source":"linear","title":"Thinking very slowly 2","reference":"ENG-2","status":"todo","detail":""},
                {"id":"hidden","source":"github","title":"Thinking-v2 private","status":"todo"}
            ]);
            state["overrides"] = json!({"one":{"status":"done","pinned":true},"hidden":{"hidden":true}});
            Ok(())
        }).unwrap();
        let result = search(&repo, &json!({"query":"thinkingv2"})).unwrap();
        assert_eq!(result["total"],1);
        assert_eq!(result["items"][0]["id"],"one");
        assert_eq!(result["items"][0]["status"],"done");
        assert_eq!(result["items"][0]["sourceStatusCode"],"todo");
        assert_eq!(result["items"][0]["pinned"],true);
        assert_eq!(search(&repo,&json!({"query":"thinkingv2","lifecycle":"active"})).unwrap()["items"][0]["id"],"two");
        assert_eq!(search(&repo,&json!({"query":"thinkingv2","includeHidden":true})).unwrap()["total"],2);
        assert!(search(&repo,&json!({"source":"../invalid"})).is_err());
        assert!(search(&repo,&json!({"includeHidden":"true"})).is_err());
        assert_eq!(normalized("Thinking-v2 / Élite"),"thinkingv2élite");
    }
    #[test]
    fn saved_view_search_matches_source_status_and_relationships_after_overrides(){
        let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();
        repo.transaction(|state|{state["items"]=json!([
            {"id":"review","source":"github","kind":"Pull request","title":"API","status":"todo","sourceStatus":"Awaiting approval","relations":["reviews"],"detail":"","createdAt":"2026-10-01T00:00:00Z"},
            {"id":"detail","source":"github","kind":"Pull request","title":"CLI","status":"todo","detail":"red elephants visit idle elephants west slowly","createdAt":"2026-10-20T00:00:00Z"}
        ]);state["overrides"]=json!({"review":{"status":"done"}});Ok(())}).unwrap();
        assert_eq!(search(&repo,&json!({"query":"approval"})).unwrap()["items"][0]["id"],"review");
        assert_eq!(search(&repo,&json!({"query":"reviews"})).unwrap()["total"],1);
        assert_eq!(search(&repo,&json!({"query":"reviews","lifecycle":"active"})).unwrap()["total"],0);
        let current=super::super::views::view(&repo).unwrap();super::super::views::update(&repo,&json!({"revision":current["revision"],"contentHash":current["contentHash"],"views":[{"id":"complete-reviews","name":"Completed reviews","parent":"github","lifecycle":"completed","filter":{"op":"all","conditions":[]},"query":"reviews","groupBy":"status"}]})).unwrap();
        let result=search(&repo,&json!({"viewId":"complete-reviews"})).unwrap();assert_eq!(result["total"],1);assert_eq!(result["items"][0]["id"],"review");assert_eq!(result["items"][0]["status"],"done");
        let current=super::super::views::view(&repo).unwrap();super::super::views::update(&repo,&json!({"revision":current["revision"],"contentHash":current["contentHash"],"views":[{"id":"older-items","name":"Created before October 10","parent":"github","lifecycle":"any","filter":{"op":"beforeDate","field":"createdAt","date":"2026-10-10"},"query":"","groupBy":"status"}]})).unwrap();
        let result=search(&repo,&json!({"viewId":"older-items"})).unwrap();assert_eq!(result["total"],1);assert_eq!(result["items"][0]["id"],"review");assert_eq!(result["items"][0]["createdAt"],"2026-10-01T00:00:00Z");
    }

}
