use super::{arr, directory, json, sid, text, uuid, Error, Repository, Result, Value};
pub fn open(repo: &Repository, p: &Value) -> Result<Value> {
    let chat = sid(text(p, "recordId"))?;
    let saved=repo.transaction(|s|{
  let record=arr(&s["sessions"]).into_iter().find(|r|r["id"]==chat).ok_or_else(|| Error::new("Select an existing chat."))?;
  if !s["chatTerminals"].is_object(){s["chatTerminals"]=json!({"version":1,"byChat":{}});}
  if !s["chatTerminals"]["byChat"][&chat].is_object(){s["chatTerminals"]["byChat"][&chat]=json!({"id":uuid(),"chatId":chat,"sessionId":"","agent":"shell","title":"Terminal","cwd":directory(text(&record,"cwd"))?});}
  Ok(s["chatTerminals"]["byChat"][&chat].clone())
 })?;
    repo.shells.open_shell(&saved)?;
    repo.transaction(|s| {
        super::guidance::complete(s, "chat.shell");
        Ok(())
    })?;
    Ok(saved)
}
pub fn action(repo: &Repository, path: &str, p: &Value) -> Result<Value> {
    if path.ends_with("/open") {
        return open(repo, p);
    }
    let id = sid(text(p, "id"))?;
    {
        let s = repo.state.lock().map_err(|_| Error::io())?;
        if !s["chatTerminals"]["byChat"]
            .as_object()
            .is_some_and(|m| m.values().any(|r| r["id"] == id))
        {
            return Err(Error::new("Chat terminal unavailable."));
        }
    }
    match path.rsplit('/').next().unwrap_or("") {
        "poll" => repo.shells.poll(
            &id,
            p["cursor"].as_u64().unwrap_or(0),
            text(p, "generation"),
        ),
        "input" => {
            let data = p["data"]
                .as_str()
                .ok_or_else(|| Error::new("Expected terminal input."))?;
            repo.shells.input(&id, data)?;
            Ok(json!({}))
        }
        "resize" => {
            let cols = p["cols"]
                .as_u64()
                .filter(|n| (10..=500).contains(n))
                .and_then(|n| u16::try_from(n).ok())
                .ok_or_else(|| Error::new("Invalid columns."))?;
            let rows = p["rows"]
                .as_u64()
                .filter(|n| (2..=200).contains(n))
                .and_then(|n| u16::try_from(n).ok())
                .ok_or_else(|| Error::new("Invalid rows."))?;
            repo.shells.resize(&id, cols, rows)?;
            Ok(json!({}))
        }
        "close" => {
            repo.shells.close(&id, text(p, "generation"))?;
            Ok(json!({}))
        }
        _ => Err(Error::new("Unknown terminal command.")),
    }
}
#[cfg(test)]
mod tests;
