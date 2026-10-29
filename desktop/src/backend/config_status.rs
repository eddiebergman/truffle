//! Content-only status for independently refreshed configuration domains.
use super::*;

fn status(document: Result<Value>, message: &str) -> Value {
    match document.and_then(|value| Ok(context_kit::sha256(&serde_json::to_vec(&value)?))) {
        Ok(token) => json!({"token":token}),
        Err(_) => json!({"error":message}),
    }
}
fn settings(repo: &Repository) -> Result<Value> {
    let document = repo.settings.view();
    if document.get("configError").is_some() {
        return Err(Error::new("Settings configuration could not load."));
    }
    Ok(json!({"settings":document,"themes":repo.settings.themes()?}))
}
pub fn view(repo: &Repository) -> Value {
    json!({"version":1,"resources":{
        "settings":status(settings(repo),"Settings configuration could not load."),
        "views":status(super::views::view(repo),"Saved views configuration could not load."),
        "pageSettings":status(super::page_settings::view(repo),"Page settings configuration could not load.")
    }})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn token<'a>(value: &'a Value, resource: &str) -> &'a Value { &value["resources"][resource]["token"] }
    fn put(path: &std::path::Path, value: &Value) { std::fs::write(path,serde_json::to_vec_pretty(value).unwrap()).unwrap(); }
    fn assert_only_changed(before: &Value, after: &Value, resource: &str) {
        for key in ["settings","views","pageSettings"] {
            if key==resource { assert_ne!(token(before,key),token(after,key)); }
            else { assert_eq!(token(before,key),token(after,key)); }
        }
    }
    #[test]
    fn stable_tokens_follow_each_domains_api_writes() {
        let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();
        let mut before=view(&repo);assert_eq!(before,view(&repo));
        for key in ["settings","views","pageSettings"] {
            let value=token(&before,key).as_str().unwrap();assert_eq!(value.len(),64);assert!(value.bytes().all(|byte|byte.is_ascii_hexdigit()));
        }
        repo.settings.apply(&json!({"revision":repo.settings.view()["revision"],"contentHash":repo.settings.view()["contentHash"],"patch":{"uiScale":120}}),false).unwrap();
        let after=view(&repo);assert_only_changed(&before,&after,"settings");before=after;
        let mut document=super::super::views::view(&repo).unwrap();document["views"][0]["name"]=json!("Changed by API");
        super::super::views::update(&repo,&json!({"revision":document["revision"],"contentHash":document["contentHash"],"views":document["views"]})).unwrap();
        let after=view(&repo);assert_only_changed(&before,&after,"views");before=after;
        let pages=super::super::page_settings::view(&repo).unwrap();
        super::super::page_settings::update(&repo,&json!({"revision":pages["revision"],"contentHash":pages["contentHash"],"page":"linear","scopes":[]})).unwrap();
        let after=view(&repo);assert_only_changed(&before,&after,"pageSettings");assert_eq!(after,view(&repo));
    }
    #[test]
    fn same_revision_files_metadata_workflow_and_theme_edits_are_detected() {
        let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();let directory=repo.settings.config_directory();
        let mut before=view(&repo);
        let preferences=PathBuf::from(text(&repo.settings.view()["files"],"preferences"));
        let mut document=super::super::config::read(&preferences).unwrap();let revision=document["revision"].clone();document["values"]["uiScale"]=json!(110);put(&preferences,&document);assert_eq!(document["revision"],revision);
        let after=view(&repo);assert_only_changed(&before,&after,"settings");before=after;
        for (name,key,value) in [("commands.json","label",json!("New label")),("settings-fields.json","label",json!("New field label"))] {
            let path=directory.join(name);let mut document=super::super::config::read(&path).unwrap();
            if name=="commands.json" {document["commands"][0][key]=value;}else{document["fields"][0][key]=value;}
            put(&path,&document);let after=view(&repo);assert_only_changed(&before,&after,"settings");before=after;
        }
        let workflow=directory.join("workflows/agent-review.md");std::fs::write(&workflow,"Custom workflow benign-private-workflow-marker").unwrap();
        let after=view(&repo);assert_only_changed(&before,&after,"settings");before=after;
        let theme=directory.join("themes/rose-pine.json");let mut document=super::super::config::read(&theme).unwrap();document["colors"]["editor.background"]=json!("#123456");put(&theme,&document);
        let after=view(&repo);assert_only_changed(&before,&after,"settings");before=after;
        let path=directory.join("views.json");let mut document=super::super::config::read(&path).unwrap();let revision=document["revision"].clone();document["views"][0]["query"]=json!("Changed externally");put(&path,&document);assert_eq!(document["revision"],revision);
        let after=view(&repo);assert_only_changed(&before,&after,"views");before=after;
        let path=directory.join("page-settings.json");let mut document=super::super::config::read(&path).unwrap();document["pages"]["github"]["scopes"]=json!(["company/project"]);put(&path,&document);
        let after=view(&repo);assert_only_changed(&before,&after,"pageSettings");assert_eq!(after,view(&repo));
    }
    #[test]
    fn resource_errors_are_independent_and_never_expose_configuration() {
        let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();let directory=repo.settings.config_directory();
        repo.settings.apply(&json!({"revision":0,"contentHash":repo.settings.view()["contentHash"],"patch":{"quickActions":[{"id":"private","title":"Private","command":"printf benign-private-command-marker"}]}}),false).unwrap();
        std::fs::write(directory.join("workflows/agent-review.md"),"benign-private-workflow-marker").unwrap();
        let initial=view(&repo);let serialized=initial.to_string();assert!(!serialized.contains("benign-private"));assert!(!serialized.contains(&directory.to_string_lossy().to_string()));
        for resource in ["views","pageSettings","settings"] {
            let path=directory.join(match resource {"views"=>"views.json","pageSettings"=>"page-settings.json",_=>"defaults.json"});let bytes=std::fs::read(&path).unwrap();std::fs::write(&path,"benign-private-malformed-marker").unwrap();
            let broken=view(&repo);assert!(broken["resources"][resource]["error"].is_string());assert!(token(&broken,resource).is_null());
            for other in ["settings","views","pageSettings"] {if other!=resource {assert_eq!(token(&broken,other),token(&initial,other));}}
            assert!(!broken.to_string().contains("benign-private"));std::fs::write(path,bytes).unwrap();
        }
    }
}
