use super::{directory, json, sessions, sid, text, uuid, Error, Repository, Result, Value};
pub fn open(repo: &Repository, p: &Value) -> Result<Value> {
    let chat = sid(text(p, "recordId"))?;
    let saved=repo.transaction(|s|{
  let record=sessions::get(s,&chat)?.clone();
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// An action on an open chat shell.
pub enum Action {
    Poll,
    Input,
    Resize,
    Close,
}
pub fn action(repo: &Repository, action: Action, p: &Value) -> Result<Value> {
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
    match action {
        Action::Poll => repo.shells.poll(
            &id,
            p["cursor"].as_u64().unwrap_or(0),
            text(p, "generation"),
        ),
        Action::Input => {
            let data = p["data"]
                .as_str()
                .ok_or_else(|| Error::new("Expected terminal input."))?;
            repo.shells.send(&id, data)?;
            Ok(json!({}))
        }
        Action::Resize => {
            repo.shells.resize(&id, p)?;
            Ok(json!({}))
        }
        Action::Close => {
            repo.shells.close(&id, text(p, "generation"))?;
            Ok(json!({}))
        }
    }
}
#[cfg(test)]
mod tests;
