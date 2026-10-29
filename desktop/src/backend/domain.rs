//! Canonical record operations and projections. No HTTP, terminal, or UI dependencies.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
pub const STATUSES: &[&str] = &[
    "todo",
    "in-progress",
    "needs-review",
    "ready-to-merge",
    "draft",
    "blocked",
    "done",
];
pub fn initialize(s: &mut Value) -> Result<()> {
    if s["version"].as_u64() != Some(1) {
        return Err(Error::new(
            "Unsupported storage version. Restore a version 1 backup.",
        ));
    }
    for key in [
        "items",
        "notes",
        "sessions",
        "changes",
        "baselineSources",
        "workLinks",
        "briefings",
        "sentryViews",
        "artifacts",
        "skillPresets",
        "skillRuns",
    ] {
        if s.get(key).is_none() {
            s[key] = json!([]);
        }
        if !s[key].is_array() {
            return Err(Error::new(format!(
                "Invalid stored {key}: expected a list. Restore a valid backup."
            )));
        }
    }
    for key in [
        "overrides",
        "itemArchive",
        "folderMappings",
        "linearLinks",
        "itemSettings",
        "snoozes",
        "changeBaselines",
        "githubReviewInbox",
        "sentryEnvironments",
        "sources",
        "skillSchedules",
    ] {
        if s.get(key).is_none() {
            s[key] = json!({});
        }
        if !s[key].is_object() {
            return Err(Error::new(format!(
                "Invalid stored {key}: expected an object. Restore a valid backup."
            )));
        }
    }
    for source in ["github", "linear", "sentry"] {
        if s["sources"].get(source).is_none() {
            s["sources"][source] =
                json!({"lastSuccess":null,"nextAttempt":0,"error":null,"failures":0});
        }
        if !s["sources"][source].is_object() {
            return Err(Error::new("Invalid stored source configuration."));
        }
    }
    for environment in s["sentryEnvironments"]
        .as_object_mut()
        .unwrap()
        .values_mut()
    {
        if !environment.is_object() {
            return Err(Error::new("Invalid stored environment cache."));
        }
        environment["running"] = json!(false);
    }
    Ok(())
}
pub fn item_ref(item: &Value) -> Value {
    let mut v = json!({});
    for key in [
        "id",
        "source",
        "title",
        "reference",
        "url",
        "project",
        "status",
        "sourceUnavailable",
        "projectId",
        "parent",
        "archivedAt",
        "sourceStatus",
        "sentryLinks",
    ] {
        v[key] = item.get(key).cloned().unwrap_or(json!(""));
    }
    v
}
pub fn catalog(s: &Value) -> BTreeMap<String, Value> {
    let mut result: BTreeMap<String, Value> = s["itemArchive"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())))
        .collect();
    for session in arr(&s["sessions"]) {
        for i in arr(&session["items"]) {
            result.entry(text(&i, "id").into()).or_insert(i);
        }
    }
    for item in arr(&s["items"]).into_iter().chain(arr(&s["notes"])) {
        result.insert(text(&item, "id").into(), item);
    }
    result
}
pub fn fingerprint(i: &Value) -> Value {
    json!({"status":i["status"],"native":i["sourceStatus"],"ci":i["ci"]["state"],"merge":i["mergeable"],"review":arr(&i["relations"]).contains(&json!("reviews"))})
}
pub fn snoozed(s: &Value, i: &Value) -> bool {
    let v = &s["snoozes"][text(i, "id")];
    if v.is_null() {
        false
    } else if v["until"].as_f64().unwrap_or(0.0) > 0.0 {
        v["until"].as_f64().unwrap() > now()
    } else {
        v["fingerprint"] == fingerprint(i)
    }
}
pub fn reasons(i: &Value) -> Vec<&'static str> {
    if i["sourceUnavailable"] == true
        || i["status"] == "done"
        || !i["archivedAt"].is_null() && i["archivedAt"] != ""
    {
        return vec![];
    }
    let roles = arr(&i["relations"]);
    let mut v = vec![];
    if i["source"] == "github" {
        if roles.contains(&json!("reviews")) && i["status"] != "draft" {
            v.push("Your review requested");
        }
        if roles.iter().any(|x| x == "authored" || x == "assigned") {
            if i["ci"]["state"] == "failed" {
                v.push("Fix failing CI");
            }
            if i["mergeable"] == "CONFLICTING" {
                v.push("Resolve merge conflicts");
            }
            if i["sourceStatus"] == "Changes requested" {
                v.push("Address requested changes");
            }
            if i["status"] == "ready-to-merge" {
                v.push("Ready to merge");
            }
        }
    } else if i["status"] == "blocked"
        && (i["source"] == "notes" || roles.contains(&json!("assigned")))
    {
        v.push("Unblock your work");
    }
    v
}
pub fn bundles(s: &Value) -> Value {
    let items = catalog(s);
    let mut groups: Vec<BTreeSet<String>> =
        items.keys().map(|k| BTreeSet::from([k.clone()])).collect();
    let refs: BTreeMap<String, String> = items
        .iter()
        .filter(|(_, v)| v["source"] == "linear")
        .map(|(k, v)| (text(v, "reference").into(), k.clone()))
        .collect();
    let mut edges = arr(&s["workLinks"])
        .into_iter()
        .map(|x| {
            arr(&x)
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    for session in arr(&s["sessions"]) {
        if !["work-review","newsletter"].contains(&text(&session,"workflow")) {
            edges.push(
                arr(&session["items"])
                    .iter()
                    .map(|v| text(v, "id").into())
                    .collect(),
            );
        }
    }
    let re = regex::Regex::new(r"\b[A-Z][A-Z0-9]*-\d+\b").unwrap();
    let sentry =
        regex::Regex::new(r"^https://(?:[^/]+\.)?sentry\.io/(?:organizations/[^/]+/)?issues/(\d+)")
            .unwrap();
    let url_key = |url: &str| {
        sentry
            .captures(url)
            .map(|c| format!("sentry-issue:{}", &c[1]))
            .unwrap_or_else(|| url.trim_end_matches('/').to_owned())
    };
    let urls: BTreeMap<_, _> = items
        .iter()
        .filter(|(_, v)| !text(v, "url").is_empty())
        .map(|(id, v)| (url_key(text(v, "url")), id.clone()))
        .collect();
    let url_re = regex::Regex::new(r"https://[^\s<>]+").unwrap();
    for (key, item) in &items {
        if item["source"] == "github" {
            let body = format!("{}\n{}", text(item, "title"), text(item, "detail"));
            for m in re.find_iter(&body) {
                if let Some(id) = refs.get(m.as_str()) {
                    edges.push(vec![key.clone(), id.clone()]);
                }
            }
            for found in url_re.find_iter(&body) {
                if let Some(id) = urls.get(&url_key(
                    found.as_str().trim_end_matches([')', '.', ',', ';']),
                )) {
                    edges.push(vec![key.clone(), id.clone()]);
                }
            }
        }
        for link in arr(&item["sentryLinks"]) {
            if let Some(id) = urls.get(&url_key(text(&link, "url"))) {
                edges.push(vec![key.clone(), id.clone()]);
            }
        }
    }
    for edge in edges {
        let ids: BTreeSet<_> = edge.into_iter().filter(|x| items.contains_key(x)).collect();
        if ids.len() < 2 {
            continue;
        }
        let mut merged = BTreeSet::new();
        groups.retain(|g| {
            if !g.is_disjoint(&ids) {
                merged.extend(g.iter().cloned());
                false
            } else {
                true
            }
        });
        groups.push(merged);
    }
    json!(groups
        .into_iter()
        .filter(|g| g.len() > 1)
        .map(|g| json!({"id":g.first(),"itemIds":g}))
        .collect::<Vec<_>>())
}
pub fn observe(s: &mut Value, source: &str, items: &[Value]) {
    let seeded = arr(&s["baselineSources"]).contains(&json!(source));
    let mut changes = arr(&s["changes"]);
    for item in items {
        if item["sourceUnavailable"] == true {
            continue;
        }
        let key = text(item, "id");
        let current = fingerprint(item);
        let old = &s["changeBaselines"][key]["value"];
        let mut messages = vec![];
        if old.is_object() {
            if current["ci"] != old["ci"] {
                match text(&current, "ci") {
                    "failed" => messages.push("CI failing".into()),
                    "passed" if old["ci"] == "failed" => messages.push("CI recovered".into()),
                    _ => (),
                }
            }
            if current["native"] != old["native"] && !text(&current, "native").is_empty() {
                messages.push(format!(
                    "{} → {}",
                    text(old, "native"),
                    text(&current, "native")
                ));
            } else if current["status"] != old["status"] {
                messages.push(format!(
                    "{} → {}",
                    text(old, "status"),
                    text(&current, "status")
                ));
            }
            if current["review"] == true && old["review"] != true {
                messages.push("Your review requested".into());
            }
            if current["merge"] == "CONFLICTING" && old["merge"] != "CONFLICTING" {
                messages.push("Merge conflict appeared".into());
            }
        } else if seeded && current["review"] == true {
            messages.push("Your review requested".into());
        }
        if !messages.is_empty() {
            changes.insert(0,json!({"id":uuid(),"itemId":key,"title":item["title"],"reference":item["reference"],"source":source,"url":item["url"],"message":messages.join(" · "),"at":now(),"read":false}));
        }
        s["changeBaselines"][key] = json!({"value":current,"seenAt":now()});
    }
    changes.truncate(500);
    s["changes"] = json!(changes);
    if !seeded {
        s["baselineSources"]
            .as_array_mut()
            .unwrap()
            .push(json!(source));
    }
}
pub fn edit(s: &mut Value, p: &Value) -> Result<()> {
    let key = text(p, "id");
    let action = text(p, "action");
    let cat = catalog(s);
    match action {
        "rename-session" => {
            let title = checked(p, "title", 180, true)?;
            let row = s["sessions"].as_array_mut().unwrap().iter_mut().find(|row| row["id"] == key).ok_or(Error::new("Session unavailable."))?;
            row["title"] = json!(title);
            row["updatedAt"] = json!(now());
        }
        "save-note" => { super::projects::save_in_state(s,p)?; }
        "delete-note" | "unlink-session" | "delete-sentry-view" => {
            let collection = match action {
                "delete-note" => "notes",
                "unlink-session" => "sessions",
                _ => "sentryViews",
            };
            s[collection] = json!(arr(&s[collection])
                .into_iter()
                .filter(|x| x["id"] != key)
                .collect::<Vec<_>>());
        }
        "archive-project" | "archive-session" | "hide-session" => {
            let collection = if action == "archive-project" {
                "notes"
            } else {
                "sessions"
            };
            let row = s[collection]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|x| x["id"] == key)
                .ok_or(Error::new("Record no longer exists."))?;
            if action == "hide-session" {
                if !p["hidden"].is_boolean() {
                    return Err(Error::new("Choose a visibility setting."));
                }
                row["hidden"] = p["hidden"].clone();
            } else {
                row["archivedAt"] = if p["restore"] == true {
                    Value::Null
                } else {
                    json!(now())
                };
            }
        }
        "override" => {
            if !cat.contains_key(key) {
                return Err(Error::new("Item no longer exists."));
            }
            let values = p["value"]
                .as_object()
                .ok_or(Error::new("Invalid override."))?;
            for (field, value) in values {
                match field.as_str() {
                    "hidden" | "pinned" if value.is_boolean() => (),
                    "status"
                        if value.is_null() || STATUSES.contains(&value.as_str().unwrap_or("")) =>
                    {
                        ()
                    }
                    _ => return Err(Error::new("Invalid override.")),
                }
                if field == "status" {
                    if let Some(project) = s["notes"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|x| x["id"] == key)
                    {
                        if !value.is_null() {
                            project["status"] = value.clone();
                        }
                        if let Some(old) = s["overrides"][key].as_object_mut() {
                            old.remove("status");
                        }
                        continue;
                    }
                }
                if !s["overrides"][key].is_object() {
                    s["overrides"][key] = json!({});
                }
                s["overrides"][key][field] = value.clone();
            }
        }
        "snooze" => {
            let item = cat
                .get(key)
                .ok_or(Error::new("Work item no longer exists."))?;
            match text(p, "mode") {
                "clear" => {
                    s["snoozes"].as_object_mut().unwrap().remove(key);
                }
                "change" => {
                    s["snoozes"][key] = json!({"fingerprint":fingerprint(item),"createdAt":now()})
                }
                "until" => {
                    let until = p["until"].as_f64().unwrap_or(0.0);
                    if !until.is_finite() || until <= now() || until >= now() + 366.0 * 86400.0 {
                        return Err(Error::new("Choose a future time within the next year."));
                    }
                    s["snoozes"][key] = json!({"until":until,"createdAt":now()});
                }
                _ => return Err(Error::new("Choose a snooze duration.")),
            }
        }
        "read-changes" => {
            super::changes::legacy_mark(s,p)?;
        }
        "link-work" | "unlink-work" => {
            let change = if action == "link-work" { super::work_links::Change::Add } else { super::work_links::Change::Remove };
            super::work_links::legacy_change(s, p, &cat, change)?;
        }
        "checkpoint" => {
            let value = json!({"done":checked(p,"done",4000,false)?,"blocked":checked(p,"blocked",4000,false)?,"next":checked(p,"next",4000,false)?,"at":now()});
            let session = s["sessions"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|x| x["id"] == key)
                .ok_or(Error::new("Session no longer exists."))?;
            session["checkpoint"] = value;
        }
        "save-briefing" => {
            let session = arr(&s["sessions"])
                .into_iter()
                .find(|x| x["id"] == key && x["workflow"] == "work-review")
                .ok_or(Error::new("Choose a Work review session."))?;
            let body = checked(p, "body", 24000, true)?;
            let mut reports = arr(&s["briefings"]);
            reports.retain(|x| x["sessionId"] != key);
            reports.insert(0,json!({"id":uuid(),"sessionId":key,"body":body,"at":now(),"sourceTimes":session["reviewSourceTimes"]}));
            reports.truncate(30);
            s["briefings"] = json!(reports);
        }
        "save-sentry-view" => {
            let name = checked(p, "name", 60, true)?;
            let filters = p["filters"]
                .as_object()
                .ok_or(Error::new("Invalid Sentry filters."))?;
            for (k, v) in filters {
                match k.as_str() {
                    "minUsers" | "minEvents" | "days"
                        if v.as_u64().is_some()
                            && v.as_u64().unwrap() <= if k == "days" { 90 } else { 100000000 } =>
                    {
                        ()
                    }
                    "level"
                        if ["", "fatal", "error", "warning", "info", "debug"]
                            .contains(&v.as_str().unwrap_or("invalid")) =>
                    {
                        ()
                    }
                    "category"
                        if ["", "error", "performance", "replay", "metric"]
                            .contains(&v.as_str().unwrap_or("invalid")) =>
                    {
                        ()
                    }
                    "environment"
                        if v.as_str().is_some() && text(&p["filters"], k).len() <= 100 =>
                    {
                        ()
                    }
                    _ => return Err(Error::new("Invalid Sentry filters.")),
                }
            }
            let id = if key.is_empty() { uuid() } else { sid(key)? };
            let mut views = arr(&s["sentryViews"]);
            views.retain(|x| x["id"] != id);
            if views.len() >= 20 {
                return Err(Error::new("Keep up to 20 saved filters."));
            }
            views.push(json!({"id":id,"name":name,"filters":filters}));
            s["sentryViews"] = json!(views);
        }
        _ => return Err(Error::new("Unknown local action.")),
    }
    Ok(())
}
