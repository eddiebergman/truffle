//! Preparation saves context; it never opens a terminal.
//! Aliases last while their saved record is retained; absence does not prove no effect.
use super::{json, now, sessions, sid, text, view_context, workflows};
use super::{Error, PathBuf, Repository, Result, Value};
const MAX_ALIASES: usize = 128;
const RECEIPT: &str = "preparationReceipt";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Quick,
    PrReview,
    Review,
    Skill,
    Recover,
    New,
    Attach,
    View,
}
impl Kind {
    const ALL: [Self; 8] = [
        Self::Quick,
        Self::PrReview,
        Self::Review,
        Self::Skill,
        Self::Recover,
        Self::New,
        Self::Attach,
        Self::View,
    ];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::PrReview => "pr-review",
            Self::Review => "review",
            Self::Skill => "skill",
            Self::Recover => "recover",
            Self::New => "new",
            Self::Attach => "attach",
            Self::View => "view",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }
    /// The command arguments: the recipe's intent plus `requestId`.
    pub fn schema(self) -> Value {
        match self {
            Self::Quick | Self::PrReview => workflows::Quick::schema(),
            Self::Review => workflows::WorkReview::schema(),
            Self::Skill => workflows::Skill::schema(),
            Self::Recover => workflows::Recover::schema(),
            Self::New => workflows::NewChat::schema(),
            Self::Attach => workflows::NewChat::attach_schema(),
            Self::View => view_context::View::schema(),
        }
    }
}
/// The command arguments for `read`.
pub fn read_schema() -> Value {
    schema(json!({}), &[])
}
pub(super) fn uuid_schema() -> Value {
    json!({"type":"string","maxLength":36,"pattern":"^[a-fA-F0-9]{8}-[a-fA-F0-9]{4}-[a-fA-F0-9]{4}-[a-fA-F0-9]{4}-[a-fA-F0-9]{12}$"})
}
/// A recipe's command arguments: its intent fields plus the required `requestId`.
pub(super) fn schema(mut properties: Value, required: &[&str]) -> Value {
    properties["requestId"] = uuid_schema();
    let required = std::iter::once("requestId")
        .chain(required.iter().copied())
        .collect::<Vec<_>>();
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}

/// One kind of preparation, built from an intent that was validated before the
/// canonical transaction starts.
pub(super) trait Recipe {
    /// The cooldown key and length. Only building a new record consumes it.
    fn admission(&self) -> Option<(String, f64)>;
    /// The saved chat to return instead of building a new one.
    fn reusable(&self, s: &Value) -> Result<Option<Value>>;
    /// Save the new record. Put a filesystem effect in `created` before making it,
    /// so that a failed commit removes it.
    fn build(
        &self,
        repo: &Repository,
        s: &mut Value,
        created: &mut Option<Created>,
    ) -> Result<Value>;
}
/// A filesystem effect of `build` that a failed preparation removes.
pub(super) enum Created {
    Folder(CreatedFolder),
    Snapshot(PathBuf),
}
// Best-effort rollback only: a crash can leave a folder, and inode checking is
// not atomic with rmdir against a concurrent rename. Never remove contents.
pub(super) struct CreatedFolder {
    path: Option<PathBuf>,
    identity: Option<(u64, u64)>,
}
impl CreatedFolder {
    fn create(path: &std::path::Path) -> Result<Self> {
        use std::os::unix::fs::MetadataExt;
        std::fs::create_dir(path)?;
        let identity = std::fs::symlink_metadata(path)
            .ok()
            .filter(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
            .map(|metadata| (metadata.dev(), metadata.ino()));
        Ok(Self {
            path: Some(path.into()),
            identity,
        })
    }
    fn cleanup(&mut self) -> bool {
        use std::os::unix::fs::MetadataExt;
        let Some(path) = self.path.take() else {
            return true;
        };
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(value) => value,
            Err(error) => return error.kind() == std::io::ErrorKind::NotFound,
        };
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || self.identity != Some((metadata.dev(), metadata.ino()))
        {
            return false;
        }
        match std::fs::remove_dir(path) {
            Ok(()) => true,
            Err(error) => error.kind() == std::io::ErrorKind::NotFound,
        }
    }
}
impl Drop for CreatedFolder {
    fn drop(&mut self) {
        self.cleanup();
    }
}
/// Create the folder at `path` and record it in `created`.
pub(super) fn create_folder(created: &mut Option<Created>, path: &std::path::Path) -> Result<()> {
    let folder = CreatedFolder::create(path)?;
    let verified = folder.identity.is_some();
    *created = Some(Created::Folder(folder));
    if !verified {
        return Err(Error::new("The new folder identity could not be verified."));
    }
    Ok(())
}
fn rollback(result: Result<Value>, created: &mut Option<Created>) -> Result<Value> {
    match (result, created) {
        (Ok(record), Some(Created::Folder(folder))) => {
            folder.path = None;
            Ok(record)
        }
        (Ok(record), _) => Ok(record),
        (Err(mut error), Some(Created::Folder(folder))) => {
            if !folder.cleanup() {
                error.message.push_str(
                    " The new folder was retained because it could not be safely removed.",
                );
            }
            Err(error)
        }
        (Err(error), Some(Created::Snapshot(path))) => {
            let _ = std::fs::remove_file(path);
            Err(error)
        }
        (Err(error), None) => Err(error),
    }
}

struct Request {
    id: String,
    kind: Kind,
    hash: String,
}
fn conflict(message: &str) -> Error {
    let mut error = Error::new(message);
    error.status = 409;
    error
}
fn exact(repo: &Repository, value: &str) -> Result<()> {
    let mut safe = value.to_owned();
    super::commands::redact_text(repo, &mut safe);
    if safe != value || value.chars().any(char::is_control) {
        return Err(Error::new(
            "Preparation identity cannot be safely projected.",
        ));
    }
    Ok(())
}
fn record_identity(repo: &Repository, record: &Value) -> Result<()> {
    let id = text(record, "id");
    if sid(id).ok().as_deref() != Some(id) {
        return Err(conflict("The saved preparation chat identity is invalid."));
    }
    exact(repo, id)
}
fn projection(repo: &Repository, id: &str) -> Result<()> {
    for value in [id, "prepared", "not-found"]
        .into_iter()
        .chain(Kind::ALL.map(Kind::name))
    {
        exact(repo, value)?;
    }
    Ok(())
}
fn aliases(record: &Value) -> Result<Vec<Value>> {
    let Some(document) = record.get("preparationRequests") else {
        return Ok(vec![]);
    };
    if document["version"] != 1 || serde_json::to_vec(document)?.len() > 64 * 1024 {
        return Err(conflict(
            "The saved preparation identities are unsupported or exceed their limit.",
        ));
    }
    let entries = document["entries"]
        .as_array()
        .filter(|rows| rows.len() <= MAX_ALIASES)
        .ok_or_else(|| conflict("The saved preparation identities are invalid."))?;
    let mut ids = std::collections::BTreeSet::new();
    for entry in entries {
        let id = sid(text(entry, "requestId"))
            .map_err(|_| conflict("The saved preparation identities are invalid."))?;
        if id != text(entry, "requestId")
            || !ids.insert(id)
            || Kind::parse(text(entry, "kind")).is_none()
            || !entry["hash"].as_str().is_some_and(|hash| {
                hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            || !entry["reused"].is_boolean()
        {
            return Err(conflict("The saved preparation identities are invalid."));
        }
    }
    Ok(entries.clone())
}
fn find(state: &Value, id: &str) -> Result<Option<(Value, Value)>> {
    let mut seen = std::collections::BTreeSet::new();
    let mut found = None;
    for record in sessions::all(state) {
        for entry in aliases(record)? {
            if !seen.insert(text(&entry, "requestId").to_owned()) {
                return Err(conflict(
                    "Preparation identities must belong to exactly one saved chat.",
                ));
            }
            if entry["requestId"] == id {
                found = Some((record.clone(), entry));
            }
        }
    }
    Ok(found)
}
fn receipt(record: &Value, entry: &Value) -> Value {
    json!({"requestId":entry["requestId"],"recordId":record["id"],"kind":entry["kind"],"status":"prepared","reused":entry["reused"]})
}
impl Request {
    fn lookup(&self, repo: &Repository, state: &Value) -> Result<Option<Value>> {
        let Some((mut record, entry)) = find(state, &self.id)? else {
            return Ok(None);
        };
        if entry["kind"] != self.kind.name() || entry["hash"] != self.hash {
            return Err(conflict("This preparation identity already belongs to different instructions. Keep its original request or start a separate deliberate preparation."));
        }
        record_identity(repo, &record)?;
        record[RECEIPT] = receipt(&record, &entry);
        Ok(Some(record))
    }
}
fn lookup(repo: &Repository, request: Option<&Request>, state: &Value) -> Result<Option<Value>> {
    request.map_or(Ok(None), |request| request.lookup(repo, state))
}
fn finish(
    repo: &Repository,
    request: Option<&Request>,
    state: &mut Value,
    record: Value,
    reused: bool,
) -> Result<Value> {
    let Some(request) = request else {
        return Ok(record);
    };
    record_identity(repo, &record)?;
    let saved = sessions::get_mut(state, text(&record, "id"))?;
    let mut entries = aliases(saved)?;
    if entries.len() >= MAX_ALIASES {
        return Err(conflict("This saved chat has reached its preparation identity limit. Existing preparations remain readable; open that saved chat instead of preparing it again."));
    }
    let entry = json!({"requestId":request.id,"kind":request.kind.name(),"hash":request.hash,"reused":reused});
    entries.push(entry.clone());
    if saved.get("preparationRequests").is_none() {
        saved["preparationRequests"] = json!({"version":1});
    }
    saved["preparationRequests"]["entries"] = json!(entries);
    if serde_json::to_vec(&saved["preparationRequests"])?.len() > 64 * 1024 {
        return Err(conflict(
            "The saved preparation identities exceed their limit.",
        ));
    }
    let mut reply = saved.clone();
    reply[RECEIPT] = receipt(saved, &entry);
    Ok(reply)
}
/// Run `recipe` in one transaction: an existing alias, then a reusable chat,
/// then admission and a new record.
fn commit(repo: &Repository, request: Option<&Request>, recipe: &impl Recipe) -> Result<Value> {
    let mut created = None;
    let result = transaction(repo, recipe.admission(), |s, admit| {
        if let Some(existing) = lookup(repo, request, s)? {
            return Ok(existing);
        }
        if let Some(existing) = recipe.reusable(s)? {
            return finish(repo, request, s, existing, true);
        }
        admit()?;
        let record = recipe.build(repo, s, &mut created)?;
        finish(repo, request, s, record, false)
    });
    rollback(result, &mut created)
}
/// Commit one preparation. `admit` checks the cooldown after the reuse checks;
/// the cooldown starts only after the commit succeeds.
fn transaction<T>(
    repo: &Repository,
    admission: Option<(String, f64)>,
    f: impl FnOnce(&mut Value, &mut dyn FnMut() -> Result<()>) -> Result<T>,
) -> Result<T> {
    let mut state = repo.state.lock().map_err(|_| Error::io())?;
    let Some((key, seconds)) = admission else {
        return repo.commit_locked(&mut state, |next| f(next, &mut || Ok(())), false);
    };
    let mut times = repo.cooldown.lock().map_err(|_| Error::io())?;
    let mut admitted = false;
    let result = repo.commit_locked(
        &mut state,
        |next| {
            f(next, &mut || {
                if !admitted && times.get(&key).copied().unwrap_or(0.0) > now() {
                    return Err(Error::new("This action just started. Give it a moment."));
                }
                admitted = true;
                Ok(())
            })
        },
        false,
    )?;
    if admitted {
        times.insert(key, now() + seconds);
    }
    drop(times);
    drop(state);
    Ok(result)
}
fn run(repo: &Repository, kind: Kind, request: Option<&Request>, intent: &Value) -> Result<Value> {
    match kind {
        Kind::Quick | Kind::PrReview => commit(
            repo,
            request,
            &workflows::Quick::validate(repo, intent, kind == Kind::PrReview)?,
        ),
        Kind::Review => commit(repo, request, &workflows::WorkReview::validate(intent)?),
        Kind::Skill => commit(repo, request, &workflows::Skill::validate(repo, intent)?),
        Kind::Recover => commit(repo, request, &workflows::Recover { intent }),
        Kind::New | Kind::Attach => commit(
            repo,
            request,
            &workflows::NewChat::validate(repo, intent, kind == Kind::Attach)?,
        ),
        Kind::View => commit(
            repo,
            request,
            &view_context::View::validate(
                repo,
                intent,
                request.map(|request| request.id.as_str()),
            )?,
        ),
    }
}
/// Save or reuse the chat for `intent` and return the saved record.
pub fn save(repo: &Repository, kind: Kind, intent: &Value) -> Result<Value> {
    run(repo, kind, None, intent)
}
fn input(kind: Kind, args: &Value) -> Result<(String, Value)> {
    super::commands::validate_arguments(&kind.schema(), args, "args")?;
    if serde_json::to_vec(args)?.len() > 64 * 1024 {
        return Err(Error::new("Preparation input exceeds 64 KiB."));
    }
    let id = sid(text(args, "requestId"))?;
    let mut payload = args.clone();
    payload.as_object_mut().unwrap().remove("requestId");
    if payload
        .as_object()
        .unwrap()
        .values()
        .any(|value| value.as_str().is_some_and(|text| text.contains('\0')))
    {
        return Err(Error::new("Preparation text cannot contain NUL."));
    }
    match kind {
        Kind::Attach => payload["sessionId"] = json!(sid(text(&payload, "sessionId"))?),
        Kind::Recover => payload["id"] = json!(sid(text(&payload, "id"))?),
        _ => {}
    }
    Ok((id, payload))
}
/// Like `save`, but idempotent by `args.requestId`: a repeat returns the first receipt.
pub fn prepare(repo: &Repository, kind: Kind, args: &Value) -> Result<Value> {
    let (id, payload) = input(kind, args)?;
    projection(repo, &id)?;
    let hash = context_kit::sha256(&serde_json::to_vec(
        &json!({"version":1,"kind":kind.name(),"intent":payload}),
    )?);
    let request = Request { id, kind, hash };
    {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        if let Some(record) = request.lookup(repo, &state)? {
            return Ok(record[RECEIPT].clone());
        }
    }
    Ok(run(repo, kind, Some(&request), &payload)?[RECEIPT].clone())
}
pub fn read(repo: &Repository, args: &Value) -> Result<Value> {
    let object = args
        .as_object()
        .filter(|args| args.len() == 1 && args.contains_key("requestId"))
        .ok_or_else(|| Error::new("Preparation lookup accepts only requestId."))?;
    let id = sid(object["requestId"]
        .as_str()
        .ok_or_else(|| Error::new("Supply the preparation UUID."))?)?;
    projection(repo, &id)?;
    let state = repo.state.lock().map_err(|_| Error::io())?;
    match find(&state, &id)? {
        Some((record, entry)) => {
            record_identity(repo, &record)?;
            Ok(receipt(&record, &entry))
        }
        None => Ok(json!({"requestId":id,"status":"not-found"})),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod admission_tests;

#[cfg(test)]
mod folder_rollback_tests;
