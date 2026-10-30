//! Captured-field edits of an existing saved chat; never opens an agent.
use super::agents::Agent;
use super::field_edit::{self, Edit, Record};
use super::{
    arr, checked, directory, domain, json, now, redaction, safe_url, sessions, sid, text, Error,
    Repository, Result, Value,
};
const FIELDS: [&str; 6] = [
    "title",
    "cwd",
    "instruction",
    "destination",
    "sessionId",
    "itemIds",
];
pub fn schema(kind: &str) -> Value {
    let id = json!({"type":"string","pattern":"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$","maxLength":36});
    let mut fields = json!({"title":{"type":"string","maxLength":180},"cwd":{"type":"string","maxLength":4096},"instruction":{"type":"string","maxLength":8000},"destination":{"type":"string","enum":["embedded","desktop"]},"sessionId":{"type":"string","maxLength":36},"itemIds":{"type":"array","maxItems":100,"items":{"type":"string","maxLength":300}}});
    let mut tokens = json!({});
    for field in FIELDS {
        tokens[field] = json!({"type":"string","pattern":"^[0-9a-fA-F]{64}$","maxLength":64});
    }
    if kind == "read" {
        return json!({"type":"object","properties":{"recordId":id},"required":["recordId"],"additionalProperties":false});
    }
    fields = json!({"type":"object","properties":fields,"additionalProperties":false});
    json!({"type":"object","properties":{"recordId":id,"patch":fields,"expected":{"type":"object","properties":tokens,"additionalProperties":false},"urls":{"type":"array","maxItems":10,"items":{"type":"string","maxLength":4096}}},"required":["recordId","patch","expected"],"additionalProperties":false})
}
fn ids(row: &Value) -> Result<Vec<String>> {
    let items = row["items"]
        .as_array()
        .ok_or_else(|| Error::new("Invalid saved chat work links."))?;
    let mut ids = Vec::new();
    for item in items {
        let id = item["id"]
            .as_str()
            .filter(|id| {
                !id.is_empty() && id.chars().count() <= 300 && !id.chars().any(char::is_control)
            })
            .ok_or_else(|| Error::new("Invalid saved chat work identity."))?;
        ids.push(id.to_owned());
    }
    ids.sort();
    ids.dedup();
    if ids.len() > 100 {
        return Err(Error::new("Saved chat exceeds 100 work links."));
    }
    Ok(ids)
}
struct Chat<'a>(&'a Value);
impl Record for Chat<'_> {
    const NAME: &'static str = "saved chat";
    const FIELDS: &'static [&'static str] = &FIELDS;
    fn id(&self) -> &str {
        text(self.0, "id")
    }
    fn projection(&self, field: &str) -> Result<Value> {
        Ok(match field {
            "itemIds" => json!(ids(self.0)?),
            "sessionId" => json!({"agent":self.0["agent"],"sessionId":self.0["sessionId"]}),
            _ => self.0[field].clone(),
        })
    }
    fn normalize(repo: &Repository, field: &str, value: &Value) -> Result<Value> {
        let patch = json!({field:value});
        match field {
            "title" => {
                let title = checked(&patch, field, 180, true)?;
                if title.chars().any(char::is_control) {
                    return Err(Error::new("Chat titles cannot contain control characters."));
                }
                Ok(json!(title))
            }
            "instruction" => Ok(json!(checked(&patch, field, 8000, false)?)),
            "cwd" => Ok(json!(directory(&checked(&patch, field, 4096, true)?)?)),
            "sessionId" => {
                let value = text(&patch, field);
                let value = if value.is_empty() {
                    String::new()
                } else {
                    sid(value)?
                };
                field_edit::exact(repo, &value)?;
                Ok(json!(value))
            }
            "itemIds" => {
                let mut collected = vec![];
                for value in value.as_array().unwrap() {
                    let id = value.as_str().unwrap();
                    if id.is_empty() || id.chars().any(char::is_control) {
                        return Err(Error::new("Invalid work identity."));
                    }
                    field_edit::exact(repo, id)?;
                    collected.push(id.to_owned());
                }
                collected.sort();
                collected.dedup();
                Ok(json!(collected))
            }
            _ => Ok(value.clone()),
        }
    }
}
fn identity(repo: &Repository, row: &Value) -> Result<Agent> {
    let id = text(row, "id");
    if sid(id)? != id {
        return Err(Error::new("Invalid saved chat identity."));
    }
    if !text(row, "sessionId").is_empty() && sid(text(row, "sessionId"))? != text(row, "sessionId")
    {
        return Err(Error::new("Invalid conversation identity."));
    }
    for value in [id, text(row, "agent"), text(row, "sessionId")] {
        field_edit::exact(repo, value)?;
    }
    for id in ids(row)? {
        field_edit::exact(repo, &id)?;
    }
    Agent::parse(text(row, "agent")).map_err(|_| Error::new("Invalid saved assistant identity."))
}
fn view(repo: &Repository, row: &Value) -> Result<Value> {
    let agent = identity(repo, row)?;
    if !["embedded", "desktop", "cmux"].contains(&text(row, "destination"))
        || !agent.supports(text(row, "destination"))
    {
        return Err(Error::new("Invalid saved launch destination."));
    }
    let mut chat = json!({"agent":row["agent"],"itemIds":ids(row)?});
    let mut redacted = false;
    for (field, bound) in [
        ("title", 180),
        ("cwd", 4096),
        ("instruction", 8000),
        ("destination", 32),
        ("sessionId", 36),
    ] {
        let value = match row.get(field) {
            Some(Value::String(value)) => value.as_str(),
            None | Some(Value::Null) if ["instruction", "sessionId"].contains(&field) => "",
            _ => return Err(Error::new("Invalid saved chat field type.")),
        };
        if value.chars().count() > bound || value.contains('\0') {
            return Err(Error::new("Saved chat exceeds its editable field limit."));
        }
        let mut display = value.to_owned();
        redaction::redact_text(repo, &mut display);
        if display != value {
            redacted = true;
        }
        if ["destination", "sessionId"].contains(&field) {
            field_edit::exact(repo, value)?;
        }
        chat[field] = json!(display);
    }
    Ok(
        json!({"recordId":row["id"],"chat":chat,"expected":field_edit::tokens(repo,&Chat(row))?,"redacted":redacted}),
    )
}
pub fn read(repo: &Repository, args: &Value) -> Result<Value> {
    super::schema::validate(&schema("read"), args, "args")?;
    let state = repo.state.lock().map_err(|_| Error::io())?;
    view(repo, sessions::get(&state, text(args, "recordId"))?)
}
fn validate<'a>(repo: &Repository, args: &Value) -> Result<Edit<Chat<'a>>> {
    super::schema::validate(&schema("update"), args, "args")?;
    if serde_json::to_vec(args)?.len() > 65536 {
        return Err(Error::new("Chat edit exceeds 64 KiB."));
    }
    let edit = Edit::new(repo, &args["patch"], &args["expected"])?;
    field_edit::exact(repo, text(args, "recordId"))?;
    if !arr(&args["urls"]).is_empty() && edit.get("itemIds").is_none() {
        return Err(Error::new(
            "Importing links requires the captured complete work-link field.",
        ));
    }
    for url in arr(&args["urls"]) {
        if safe_url(url.as_str().unwrap())?.is_empty() {
            return Err(Error::new("Supply nonempty work URLs."));
        }
    }
    Ok(edit)
}
fn association(
    state: &Value,
    current: &Value,
    agent: Agent,
    edit: &Edit<Chat>,
    repo: &Repository,
) -> Result<()> {
    if let Some(value) = edit.get("sessionId") {
        let old = text(current, "sessionId");
        if !old.is_empty() && value.as_str() != Some(old) {
            return Err(field_edit::conflict(
                "An attached conversation identity cannot be changed.",
            ));
        }
        if value != ""
            && sessions::find(state, |row| {
                row["id"] != current["id"]
                    && row["agent"] == current["agent"]
                    && row["sessionId"] == *value
            })
            .is_some()
        {
            return Err(field_edit::conflict(
                "This conversation is already associated with another saved chat.",
            ));
        }
    }
    let destination = edit
        .get("destination")
        .unwrap_or_else(|| &current["destination"])
        .as_str()
        .unwrap_or("");
    field_edit::exact(repo, destination)?;
    if !agent.supports(destination) {
        return Err(Error::new("Claude chats require the embedded terminal."));
    }
    Ok(())
}
/// The requested work links plus the imported ones, as sorted IDs and stored references.
/// A link the chat already has keeps its stored reference.
fn links(
    repo: &Repository,
    state: &Value,
    current: &Value,
    requested: &Value,
    imported: &[Value],
) -> Result<(Value, Value)> {
    let mut requested = arr(requested);
    for item in imported {
        requested.push(item["id"].clone());
    }
    requested.sort_by_key(Value::to_string);
    requested.dedup();
    if requested.len() > 100 {
        return Err(Error::new("Choose up to 100 linked work items."));
    }
    let mut catalog = domain::catalog(state);
    for item in imported {
        catalog
            .entry(text(item, "id").to_owned())
            .or_insert_with(|| item.clone());
    }
    let mut linked = vec![];
    for value in &requested {
        let id = value
            .as_str()
            .ok_or_else(|| Error::new("Invalid imported work identity."))?;
        field_edit::exact(repo, id)?;
        let item = catalog
            .get(id)
            .ok_or_else(|| Error::new("A linked work item no longer exists."))?;
        linked.push(
            arr(&current["items"])
                .into_iter()
                .find(|old| old["id"] == *value)
                .unwrap_or_else(|| domain::item_ref(item)),
        );
    }
    Ok((json!(requested), json!(linked)))
}
fn apply(
    repo: &Repository,
    state: &mut Value,
    args: &Value,
    edit: &Edit<Chat>,
    imported: &[Value],
) -> Result<Value> {
    let current = sessions::get(state, text(args, "recordId"))?.clone();
    let agent = identity(repo, &current)?;
    let mut next = current.clone();
    let changed = edit.apply(&Chat(&current), |field, value| {
        if field == "itemIds" {
            let (requested, linked) = links(repo, state, &current, value, imported)?;
            if json!(ids(&current)?) == requested {
                return Ok(false);
            }
            next["items"] = linked;
            return Ok(true);
        }
        let old = if field == "sessionId" {
            json!(text(&current, "sessionId"))
        } else {
            current[field].clone()
        };
        if old == *value {
            return Ok(false);
        }
        next[field] = value.clone();
        if field == "sessionId" && value != "" {
            next["launchNew"] = json!(false);
        }
        Ok(true)
    })?;
    association(state, &current, agent, edit, repo)?;
    if !changed.is_empty() {
        next["updatedAt"] = json!(now());
    }
    identity(repo, &next)?;
    let expected = field_edit::tokens(repo, &Chat(&next))?;
    // Provider reads cannot replace newer cached source data merely to associate it.
    for item in imported {
        if !domain::catalog(state).contains_key(text(item, "id")) {
            state["items"]
                .as_array_mut()
                .ok_or_else(Error::io)?
                .push(item.clone());
        }
    }
    *sessions::get_mut(state, text(args, "recordId"))? = next;
    Ok(json!({"recordId":current["id"],"saved":true,"changedFields":changed,"expected":expected}))
}
pub fn update(repo: &Repository, args: &Value) -> Result<Value> {
    update_with(repo, args, |url| repo.providers.resolve(url))
}
fn update_with(
    repo: &Repository,
    args: &Value,
    resolve: impl Fn(&str) -> Result<Value>,
) -> Result<Value> {
    let edit = validate(repo, args)?;
    check_current(
        repo,
        &*repo.state.lock().map_err(|_| Error::io())?,
        args,
        &edit,
    )?;
    let mut imported = vec![];
    for url in arr(&args["urls"]) {
        let url = safe_url(url.as_str().unwrap())?;
        let cached = {
            let state = repo.state.lock().map_err(|_| Error::io())?;
            domain::catalog(&state)
                .into_values()
                .find(|row| row["url"] == url)
        };
        imported.push(match cached {
            Some(row) => row,
            None => resolve(&url)?,
        });
    }
    repo.transaction_if_changed(|state| apply(repo, state, args, &edit, &imported))
}
fn check_current(repo: &Repository, state: &Value, args: &Value, edit: &Edit<Chat>) -> Result<()> {
    let row = sessions::get(state, text(args, "recordId"))?;
    let agent = identity(repo, row)?;
    edit.check(&Chat(row))?;
    association(state, row, agent, edit, repo)
}
pub(super) fn reject_unchecked_association(state: &Value, args: &Value) -> Result<()> {
    if args.get("id").is_some_and(|value| !value.is_string()) {
        return Err(Error::new("Invalid saved chat ID."));
    }
    if !text(args, "sessionId").is_empty()
        && sessions::find(state, |row| {
            row["agent"] == args["agent"] && row["sessionId"] == args["sessionId"]
        })
        .is_some()
    {
        return Err(field_edit::conflict(
            "This conversation is already saved. Reopen Manage to capture current chat fields.",
        ));
    }
    Ok(())
}
/// Old edit transports require the same captured patch; registration is a separate owner.
pub(super) fn legacy(repo: &Repository, args: &Value) -> Result<Value> {
    let object = args
        .as_object()
        .ok_or_else(|| Error::new("Invalid chat edit."))?;
    if object
        .keys()
        .any(|key| !["action", "id", "patch", "expected", "urls"].contains(&key.as_str()))
    {
        return Err(field_edit::conflict(
            "Reopen Manage to capture current chat fields before editing.",
        ));
    }
    let request = json!({"recordId":args["id"],"patch":args["patch"],"expected":args["expected"],"urls":args.get("urls").cloned().unwrap_or(json!([]))});
    update(repo, &request)?;
    let state = repo.state.lock().map_err(|_| Error::io())?;
    Ok(sessions::get(&state, text(args, "id"))?.clone())
}

#[cfg(test)]
mod tests;
