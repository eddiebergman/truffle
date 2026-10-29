use super::*;
use std::sync::Mutex;

static WRITE_LOCK: Mutex<()> = Mutex::new(());
const PROVIDERS: &[&str] = &["linear", "github", "sentry"];

fn validate(doc: &Value) -> Result<()> {
    if doc["version"] != 1
        || doc["revision"].as_u64().is_none()
        || doc.as_object().is_none_or(|o| {
            o.keys()
                .any(|k| !["version", "revision", "pages"].contains(&k.as_str()))
        })
    {
        return Err(Error::new("Invalid page settings document."));
    }
    let pages = doc["pages"]
        .as_object()
        .ok_or(Error::new("Page settings must contain pages."))?;
    if pages.len() != PROVIDERS.len() || pages.keys().any(|p| !PROVIDERS.contains(&p.as_str())) {
        return Err(Error::new("Unknown page settings."));
    }
    for page in pages.values() {
        if page
            .as_object()
            .is_none_or(|p| p.len() != 1 || !p.contains_key("scopes"))
        {
            return Err(Error::new("Unknown page setting."));
        }
        let scopes = &page["scopes"];
        if !scopes.is_null()
            && scopes.as_array().is_none_or(|a| {
                a.len() > 1000
                    || a.iter()
                        .any(|s| s.as_str().is_none_or(|s| s.len() > 500 || s.contains('\0')))
                    || a.iter()
                        .filter_map(Value::as_str)
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        != a.len()
            })
        {
            return Err(Error::new(
                "Choose at most 1000 distinct projects or repositories.",
            ));
        }
    }
    Ok(())
}
fn read_document(path: &std::path::Path) -> Result<Value> {
    let bytes=context_kit::read_bounded(path,512*1024).map_err(|_|Error::new("Cannot read page settings configuration."))?;
    let mut document:Value=serde_json::from_slice(&bytes).map_err(|_|Error::new("Invalid page settings document."))?;
    validate(&document)?;
    document["contentHash"]=json!(context_kit::sha256(&bytes));
    Ok(document)
}
pub fn view(repo: &Repository) -> Result<Value> {
    read_document(&repo.settings.config_directory().join("page-settings.json"))
}

// Match the page gear's scopeKey, rather than saved-view predicate normalization.
fn cached_scope(item: &Value, page: &str) -> Result<String> {
    if item.get("project").is_some_and(|value| !value.is_null() && !value.is_string())
        || page == "github" && item.get("url").is_some_and(|value| !value.is_null() && !value.is_string())
    {
        return Err(Error::new("Cached visibility scope metadata is invalid."));
    }
    let fallback = if page == "github" { "Unknown repository" } else { "No project" };
    let project = text(item, "project");
    let key = if page == "github" {
        text(item, "url").strip_prefix("https://github.com/").and_then(|path| {
            let mut parts = path.split('/');
            let owner = parts.next()?;
            let repo = parts.next()?;
            parts.next()?;
            (!owner.is_empty() && !repo.is_empty()).then(|| format!("{owner}/{repo}"))
        }).unwrap_or_else(|| if project.is_empty() { fallback.into() } else { project.into() })
    } else if project.is_empty() { fallback.into() } else { project.into() };
    if key.len() > 500 || key.contains('\0') {
        return Err(Error::new("Cached visibility scopes exceed their supported bounds."));
    }
    Ok(key)
}

pub fn scopes(repo: &Repository, args: &Value) -> Result<Value> {
    if args.as_object().is_none_or(|args| args.keys().any(|key| !["page", "query", "offset", "snapshot"].contains(&key.as_str()))) {
        return Err(Error::new("Supply page, query, offset and snapshot only."));
    }
    let page = args["page"].as_str().filter(|page| PROVIDERS.contains(page)).ok_or(Error::new("Choose a supported page."))?;
    let query = match args.get("query") {
        Some(Value::String(query)) if query.len() <= 500 && !query.contains('\0') => query.as_str(),
        None => "",
        _ => return Err(Error::new("Supply a visibility search of at most 500 bytes.")),
    };
    let offset = match args.get("offset") {
        Some(value) => value.as_u64().filter(|offset| *offset <= 4096).ok_or(Error::new("Choose a visibility offset from 0 to 4096."))? as usize,
        None => 0,
    };
    let supplied_snapshot = match args.get("snapshot") {
        Some(Value::String(value)) if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) => Some(value.as_str()),
        None => None,
        _ => return Err(Error::new("Supply the exact visibility snapshot.")),
    };
    let current = view(repo)?;
    let selected = &current["pages"][page]["scopes"];
    let mut choices = std::collections::BTreeMap::new();
    {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        for item in state["items"].as_array().ok_or(Error::new("Cached work is unavailable."))? {
            if item["source"] != page { continue; }
            choices.insert(cached_scope(item, page)?, true);
            if choices.len() > 4096 { return Err(Error::new("Visibility choices exceed 4096 keys.")); }
        }
    }
    for key in selected.as_array().into_iter().flatten() {
        choices.entry(key.as_str().unwrap().to_owned()).or_insert(false);
        if choices.len() > 4096 { return Err(Error::new("Visibility choices exceed 4096 keys.")); }
    }
    // This owner serves the named read. Check the complete union before a query
    // or page can hide an unsafe actionable key behind a count or snapshot.
    for key in choices.keys() {
        let mut projected = key.clone();
        super::commands::redact_text(repo, &mut projected);
        if projected != *key {
            return Err(Error::new("Page visibility scopes cannot be safely projected."));
        }
    }
    let rows = choices.into_iter().map(|(key, cached)| {
        let visible = selected.is_null() || selected.as_array().unwrap().iter().any(|selected| selected == &key);
        json!({"key":key,"cached":cached,"visible":visible})
    }).collect::<Vec<_>>();
    let bytes = serde_json::to_vec(&rows)?;
    if bytes.len() > 2 * 1024 * 1024 { return Err(Error::new("Visibility choices exceed 2 MiB.")); }
    let snapshot = context_kit::sha256(&serde_json::to_vec(&json!({"page":page,"query":query,"revision":current["revision"],"contentHash":current["contentHash"],"choices":rows}))?);
    if offset > 0 && supplied_snapshot.is_none() || supplied_snapshot.is_some_and(|value| value != snapshot) {
        let mut error = Error::new("Visibility choices changed. Restart pages.scopes.read without offset or snapshot.");
        error.status = 409;
        return Err(error);
    }
    let query = query.to_lowercase();
    let matches = rows.into_iter().filter(|row| text(row, "key").to_lowercase().contains(&query)).collect::<Vec<_>>();
    let total = matches.len();
    let mode = if selected.is_null() { "all" } else if selected.as_array().unwrap().is_empty() { "none" } else { "selected" };
    Ok(json!({"page":page,"scope":"cached","mode":mode,"revision":current["revision"],"contentHash":current["contentHash"],"snapshot":snapshot,
        "choices":matches.into_iter().skip(offset).take(100).collect::<Vec<_>>(),"total":total,"offset":offset,"nextOffset":(offset + 100 < total).then_some(offset + 100)}))
}

pub fn update(repo: &Repository, payload: &Value) -> Result<Value> {
    let _guard = WRITE_LOCK.lock().unwrap();
    let current = view(repo)?;
    let next = patch(&current, payload)?;
    let path = repo.settings.config_directory().join("page-settings.json");
    let temp = path.with_extension("tmp");
    let bytes=serde_json::to_vec_pretty(&next)?;
    super::http::private_write(&temp, &bytes)?;
    std::fs::rename(temp, path)?;
    let mut response=next;
    response["contentHash"]=json!(context_kit::sha256(&bytes));
    Ok(response)
}
fn patch(current: &Value, payload: &Value) -> Result<Value> {
    if payload.as_object().is_none_or(|p| p.keys().any(|k| !["revision","contentHash","page","scopes"].contains(&k.as_str()))) {
        return Err(Error::new("Supply revision, contentHash, page and scopes only."));
    }
    if payload["revision"].as_u64().is_none() || payload["revision"] != current["revision"]
        || payload["contentHash"].as_str().is_none_or(|hash|hash.len()!=64 || !hash.bytes().all(|byte|byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
        || payload["contentHash"] != current["contentHash"] {
        let mut error=Error::new("Page settings changed. Reopen this page's settings.");error.status=409;return Err(error);
    }
    let page = text(payload, "page");
    if !PROVIDERS.contains(&page) || payload.get("scopes").is_none() {
        return Err(Error::new("Unknown page setting."));
    }
    let mut next = current.clone();
    next.as_object_mut().unwrap().remove("contentHash");
    next["pages"][page]["scopes"] = payload["scopes"].clone();
    next["revision"] = json!(current["revision"]
        .as_u64()
        .unwrap()
        .checked_add(1)
        .ok_or(Error::new("Page settings revision is exhausted."))?);
    validate(&next)?;
    if serde_json::to_vec(&next)?.len() > 128 * 1024 {
        return Err(Error::new("Page settings exceed 128 KiB."));
    }
    Ok(next)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn payload(current: &Value, scopes: Value) -> Value { json!({"revision":current["revision"],"contentHash":current["contentHash"],"page":"linear","scopes":scopes}) }

    #[test]
    fn page_scopes_match_gear_keys_without_saved_view_normalization() {
        for (page, item, expected) in [
            ("github",json!({"url":"https://github.com/Owner/Repo/pull/7","project":"Wrong"}),"Owner/Repo"),
            ("github",json!({"url":"https://github.com/Owner/Repo/","project":"Wrong"}),"Owner/Repo"),
            ("github",json!({"url":"https://github.com/Owner/Repo","project":"Fallback"}),"Fallback"),
            ("github",json!({"url":"https://github.com//Repo/pull/7","project":"Fallback"}),"Fallback"),
            ("github",json!({"url":"https://example.invalid/Owner/Repo/pull/7","project":""}),"Unknown repository"),
            ("linear",json!({"project":"Mixed Case / 空"}),"Mixed Case / 空"),
            ("sentry",json!({"project":""}),"No project"),
            ("linear",json!({}),"No project"),
        ] { assert_eq!(cached_scope(&item,page).unwrap(),expected); }
        for item in [json!({"project":"x".repeat(501)}),json!({"project":"bad\0key"})] {
            assert!(cached_scope(&item,"linear").is_err());
        }
        for (page,item) in [("linear",json!({"project":42})),("sentry",json!({"project":{"private":"do not echo"}})),("github",json!({"url":42,"project":"Fallback"}))] {
            assert_eq!(cached_scope(&item,page).unwrap_err().message,"Cached visibility scope metadata is invalid.");
        }
    }

    #[test]
    fn page_scopes_enforce_complete_catalog_bounds_without_silent_truncation() {
        let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();
        for keys in [
            (0..4097).map(|i|json!({"id":format!("linear:{i}"),"source":"linear","project":format!("Scope {i}")})).collect::<Vec<_>>(),
            (0..4096).map(|i|json!({"id":format!("linear:{i}"),"source":"linear","project":format!("{}-{i:04}","x".repeat(495))})).collect::<Vec<_>>(),
        ] {
            repo.transaction(|state|{state["items"]=json!(keys);Ok(())}).unwrap();
            let before=std::fs::read(&repo.data).unwrap();
            assert!(scopes(&repo,&json!({"page":"linear","query":"no matching rows"})).is_err());
            assert_eq!(std::fs::read(&repo.data).unwrap(),before);
        }
    }
    #[test]
    fn visibility_updates_require_current_revision_and_file_content_hash() {
        let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();
        let path=repo.settings.config_directory().join("page-settings.json");let baseline=view(&repo).unwrap();
        let mut external=super::super::config::read(&path).unwrap();external["pages"]["github"]["scopes"]=json!(["external/project"]);
        std::fs::write(&path,serde_json::to_vec_pretty(&external).unwrap()).unwrap();let saved=std::fs::read(&path).unwrap();
        let stale=update(&repo,&payload(&baseline,json!([]))).unwrap_err();assert_eq!(stale.status,409);assert_eq!(std::fs::read(&path).unwrap(),saved);
        let fresh=view(&repo).unwrap();assert_eq!(fresh["revision"],baseline["revision"]);assert_ne!(fresh["contentHash"],baseline["contentHash"]);
        let none=update(&repo,&payload(&fresh,json!([]))).unwrap();assert_eq!(none["pages"]["linear"]["scopes"],json!([]));assert_eq!(none["pages"]["github"]["scopes"],json!(["external/project"]));assert_ne!(none["contentHash"],fresh["contentHash"]);
        let persisted=super::super::config::read(&path).unwrap();assert!(persisted.get("contentHash").is_none());assert_eq!(persisted.as_object().unwrap().len(),3);assert_eq!(none,view(&repo).unwrap());
        let selected=update(&repo,&payload(&none,json!(["Latency"]))).unwrap();assert_eq!(selected["pages"]["linear"]["scopes"],json!(["Latency"]));
        let all=update(&repo,&payload(&selected,Value::Null)).unwrap();assert!(all["pages"]["linear"]["scopes"].is_null());
        assert_eq!(update(&repo,&payload(&selected,json!([]))).unwrap_err().status,409);
    }
    #[test]
    fn invalid_or_extra_fields_never_modify_the_document() {
        let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();
        let path=repo.settings.config_directory().join("page-settings.json");let current=view(&repo).unwrap();let saved=std::fs::read(&path).unwrap();
        let mut extra=payload(&current,json!([]));extra["token"]=json!("benign-secret-marker");
        let mut unknown=payload(&current,json!([]));unknown["page"]=json!("unknown");
        for invalid in [payload(&current,json!(["x","x"])),payload(&current,json!([true])),payload(&current,json!("not-an-array")),extra,unknown] {
            assert!(update(&repo,&invalid).is_err());assert_eq!(std::fs::read(&path).unwrap(),saved);
        }
        for key in ["revision","contentHash","page","scopes"] {
            let mut invalid=payload(&current,json!([]));invalid.as_object_mut().unwrap().remove(key);
            let error=update(&repo,&invalid).unwrap_err();if ["revision","contentHash"].contains(&key) {assert_eq!(error.status,409);}assert_eq!(std::fs::read(&path).unwrap(),saved);
        }
        let mut invalid=payload(&current,json!([]));invalid["contentHash"]=json!("not-a-hash");assert!(update(&repo,&invalid).is_err());assert_eq!(std::fs::read(&path).unwrap(),saved);
        let mut invalid=super::super::config::read(&path).unwrap();invalid["contentHash"]=current["contentHash"].clone();std::fs::write(&path,serde_json::to_vec(&invalid).unwrap()).unwrap();assert!(view(&repo).is_err());
    }
}
