//! Content-only status for independently refreshed configuration domains.
use super::{json, Error, Repository, Result, Value};

fn status(document: Result<Value>, message: &str) -> Value {
    document
        .and_then(|value| Ok(context_kit::sha256(&serde_json::to_vec(&value)?)))
        .map_or_else(|_| json!({"error":message}), |token| json!({"token":token}))
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
mod tests;
