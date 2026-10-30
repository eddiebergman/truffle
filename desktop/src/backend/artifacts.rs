use super::agents::Agent;
use super::field_edit::{self, Edit, Record};
use super::{
    arr, checked, expand, json, now, redaction, safe_url, sessions, sid, text, Error, PathBuf,
    Repository, Result, Value,
};
use sha2::{Digest, Sha256};
use std::path::{Component, Path};
pub fn presets(repo: &Repository, state: &Value) -> Result<Vec<Value>> {
    let entries = super::skill_presets::load(repo)?.overlay(state)?;
    let mut result = vec![];
    for entry in &entries {
        let mut v = entry.clone();
        let cwd = expand(text(entry, "cwd"));
        let supplied_skill = expand(text(entry, "skill"));
        let skill = cwd.join(&supplied_skill);
        let reports = cwd.join(entry["reports"].as_str().unwrap_or("reports"));
        let reports_path = entry["reports"].as_str().unwrap_or("reports");
        if Path::new(reports_path).is_absolute()
            || Path::new(reports_path)
                .components()
                .any(|c| c == Component::ParentDir)
        {
            return Err(Error::new("Reports must stay within the preset folder."));
        }
        let supplied = supplied_skill.as_path();
        if supplied.components().any(|c| c == Component::ParentDir)
            || supplied.is_absolute() && (skill.extension().and_then(|s| s.to_str()) != Some("md"))
        {
            return Err(Error::new("Choose an existing absolute Markdown skill file, or a path within the working folder."));
        }
        v["cwd"] = json!(cwd);
        v["skill"] = json!(skill);
        v["reports"] = json!(reports);
        v["agent"] = json!(Agent::parse(entry["agent"].as_str().unwrap_or("claude"))?.name());
        v["available"] = json!(cwd.is_dir() && skill.is_file());
        result.push(v);
    }
    Ok(result)
}
pub fn roots(repo: &Repository, state: &Value) -> Result<Vec<PathBuf>> {
    let presets = presets(repo, state)?;
    Ok(presets
        .iter()
        .map(|p| expand(text(p, "reports")))
        .chain(sessions::all(state).iter().map(|s| expand(text(s, "cwd"))))
        .chain([private_root(repo)])
        .filter_map(|p| p.canonicalize().ok())
        .collect())
}
fn private_root(repo: &Repository) -> PathBuf {
    repo.data.parent().unwrap().join("artifacts")
}
/// Write `bytes` once under the private artifact root and return the stored path.
pub fn store_bytes(repo: &Repository, name: &str, bytes: &[u8]) -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let directory = private_root(repo);
    std::fs::create_dir_all(&directory)?;
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
    let stored = directory.join(name);
    if !stored.exists() {
        super::http::private_write(&stored, bytes)?;
    }
    Ok(stored)
}
/// Whether a stored or catalog artifact entry is linked to chat `chat`.
pub fn linked(entry: &Value, chat: &str) -> bool {
    arr(&entry["sessionIds"]).contains(&json!(chat))
}
/// Registered artifacts linked to chat `chat`, in stored order.
pub fn for_chat(state: &Value, chat: &str) -> Vec<Value> {
    arr(&state["artifacts"])
        .into_iter()
        .filter(|entry| linked(entry, chat))
        .collect()
}
/// Make `ids` the exact set of registered artifacts linked to chat `chat`.
pub fn relink(state: &mut Value, chat: &str, ids: &[Value]) -> Result<()> {
    let mut artifacts = arr(&state["artifacts"]);
    if ids
        .iter()
        .any(|id| !id.is_string() || !artifacts.iter().any(|x| x["id"] == *id))
    {
        return Err(Error::new(
            "Attach artifact files before replacing their session links.",
        ));
    }
    for artifact in &mut artifacts {
        let mut sessions = arr(&artifact["sessionIds"]);
        sessions.retain(|x| x != chat);
        if ids.contains(&artifact["id"]) {
            sessions.push(json!(chat));
        }
        artifact["sessionIds"] = json!(sessions);
    }
    state["artifacts"] = json!(artifacts);
    Ok(())
}
pub fn kind(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_lowercase().as_str() {
        "html" => Some("text/html; charset=utf-8"),
        "md" | "txt" | "csv" | "json" | "py" => Some("text/plain; charset=utf-8"),
        "pdf" => Some("application/pdf"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}
pub fn file(path: &str, roots: &[PathBuf]) -> Result<PathBuf> {
    let path = expand(path)
        .canonicalize()
        .map_err(|_| Error::new("Artifact file unavailable."))?;
    if !roots.iter().any(|root| {
        path.strip_prefix(root).is_ok_and(|p| {
            !p.components()
                .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
        })
    }) || kind(&path).is_none()
        || !path.is_file()
    {
        return Err(Error::new(
            "Choose a supported artifact file inside a session or newsletter reports folder.",
        ));
    }
    if path.metadata()?.len() > 10 * 1024 * 1024 {
        return Err(Error::new("Artifacts must be 10 MiB or smaller."));
    }
    Ok(path)
}
pub fn id(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))[..24].into()
}
pub fn catalog(repo: &Repository, s: &Value) -> Result<Value> {
    let presets = presets(repo, s)?;
    let roots = roots(repo, s)?;
    let mut entries = arr(&s["artifacts"])
        .into_iter()
        .map(|x| (text(&x, "id").to_string(), x))
        .collect::<std::collections::BTreeMap<_, _>>();
    // A shared reports folder cannot identify which skill produced a file.
    // Registered artifacts carry their own provenance; discovery is only a
    // compatibility path for folders with exactly one owner.
    let mut report_owners = std::collections::BTreeMap::new();
    for preset in &presets {
        if let Ok(root) = expand(text(preset, "reports")).canonicalize() {
            *report_owners.entry(root).or_insert(0usize) += 1;
        }
    }
    for preset in &presets {
        let Ok(root) = expand(text(preset, "reports")).canonicalize() else {
            continue;
        };
        if report_owners.get(&root) != Some(&1) {
            continue;
        }
        let Ok(dirs) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut dirs = dirs
            .filter_map(std::result::Result::ok)
            .filter(|x| x.file_name().to_string_lossy().len() == 10 && x.path().is_dir())
            .collect::<Vec<_>>();
        dirs.sort_by_key(|x| std::cmp::Reverse(x.file_name()));
        for dir in dirs.into_iter().take(365) {
            for filename in ["report.html", "report.md", "audit.html"] {
                let Ok(path) = file(
                    &dir.path().join(filename).to_string_lossy(),
                    std::slice::from_ref(&root),
                ) else {
                    continue;
                };
                let path = path.to_string_lossy().to_string();
                if entries
                    .values()
                    .any(|entry| entry["sourcePath"] == path || entry["path"] == path)
                {
                    continue;
                }
                let key = id(&path);
                if entries.contains_key(&key) {
                    continue;
                }
                let title = format!(
                    "{} · {}{}",
                    text(preset, "title"),
                    dir.file_name().to_string_lossy(),
                    if filename == "audit.html" {
                        " · Audit"
                    } else {
                        ""
                    }
                );
                entries.insert(key.clone(),json!({"id":key,"title":title,"path":path,"presetId":preset["id"],"role":"skill-output","sessionIds":[],"at":std::fs::metadata(&path)?.modified()?.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64(),"format":filename.rsplit('.').next(),"discovered":true}));
            }
        }
    }
    let mut entries = entries.into_values().collect::<Vec<_>>();
    for entry in &mut entries {
        if text(entry, "path").is_empty() {
            entry["available"] = json!(true);
        } else {
            match file(text(entry, "path"), &roots) {
                Ok(path) => {
                    entry["available"] = json!(true);
                    entry["size"] = json!(path.metadata()?.len());
                    entry["format"] = json!(path
                        .extension()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase());
                }
                Err(_) => entry["available"] = json!(false),
            }
        }
    }
    entries.sort_by(|a, b| {
        b["at"]
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&a["at"].as_f64().unwrap_or(0.0))
    });
    Ok(json!({"presets":presets,"artifacts":entries,"warnings":[]}))
}
// Metadata edits are separate from file/URL registration and preserve the
// canonical immutable identity and provenance fields without opening the file.
const METADATA_CHANGED: &str =
    "Artifact metadata changed. Read it again and reapply only the intended changes.";
fn metadata_keys(value: &Value, allowed: &[&str]) -> Result<()> {
    let fields = value
        .as_object()
        .ok_or_else(|| Error::new("Artifact metadata must be an object."))?;
    if fields.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(Error::new("Unknown artifact metadata field."));
    }
    Ok(())
}
fn metadata_id(value: &Value) -> Result<&str> {
    value
        .as_str()
        .filter(|id| id.len() == 24 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| Error::new("Supply the exact artifact ID."))
}
fn metadata_links(value: &Value) -> Result<Vec<String>> {
    let values = value
        .as_array()
        .filter(|values| values.len() <= 100)
        .ok_or_else(|| {
            Error::new("Artifact metadata supports at most 100 complete session links.")
        })?;
    let mut ids = vec![];
    for value in values {
        let id = value
            .as_str()
            .ok_or_else(|| Error::new("Invalid artifact session ID."))?;
        sid(id)?;
        ids.push(id.to_owned());
    }
    ids.sort();
    ids.dedup();
    Ok(ids)
}
/// The editable metadata of a registered or discovered artifact.
struct Metadata<'a> {
    id: String,
    values: Value,
    /// The fingerprint of a discovered artifact, so its tokens expire when discovery changes.
    discovery: Option<&'a str>,
}
impl<'a> Metadata<'a> {
    fn new(entry: &Value, discovery: Option<&'a str>) -> Result<Self> {
        let title = entry["title"].as_str().unwrap_or("Artifact");
        if title.chars().count() > 300 {
            return Err(Error::new(
                "Stored artifact title exceeds its editable limit.",
            ));
        }
        let links = if entry.get("sessionIds").is_some() {
            metadata_links(&entry["sessionIds"])?
        } else {
            vec![]
        };
        Ok(Self {
            id: metadata_id(&entry["id"])?.to_owned(),
            values: json!({"title":title,"sessionIds":links}),
            discovery,
        })
    }
}
impl Record for Metadata<'_> {
    const NAME: &'static str = "artifact";
    const FIELDS: &'static [&'static str] = &["title", "sessionIds"];
    fn id(&self) -> &str {
        &self.id
    }
    fn projection(&self, field: &str) -> Result<Value> {
        Ok(json!({"value":self.values[field],"discovery":self.discovery}))
    }
    fn normalize(repo: &Repository, field: &str, value: &Value) -> Result<Value> {
        if field == "title" {
            if !value.is_string() {
                return Err(Error::new("Artifact title must be text."));
            }
            return Ok(json!(checked(
                &json!({"title":value}),
                "title",
                300,
                false
            )?));
        }
        let ids = metadata_links(value)?;
        for id in &ids {
            field_edit::exact(repo, id)?;
        }
        Ok(json!(ids))
    }
}
fn metadata_view(repo: &Repository, entry: &Value, discovery: Option<&str>) -> Result<Value> {
    let metadata = Metadata::new(entry, discovery)?;
    let mut title = text(&metadata.values, "title").to_owned();
    redaction::redact_text(repo, &mut title);
    if title.chars().count() > 300 {
        return Err(Error::new(
            "Artifact title cannot be safely projected within its limit.",
        ));
    }
    for id in arr(&metadata.values["sessionIds"]) {
        field_edit::exact(repo, id.as_str().unwrap())?;
    }
    let kind = if discovery.is_some() {
        "discovered"
    } else {
        "registered"
    };
    field_edit::exact(repo, kind)?;
    let mut origin = json!({"type":kind});
    if let Some(fingerprint) = discovery {
        field_edit::exact(repo, fingerprint)?;
        origin["fingerprint"] = json!(fingerprint);
    }
    let expected = field_edit::tokens(repo, &metadata)?;
    Ok(
        json!({"artifact":{"id":metadata.id,"title":title,"sessionIds":metadata.values["sessionIds"]},"expected":expected,"origin":origin,"scope":"local"}),
    )
}
fn metadata_patch<'a>(repo: &Repository, args: &Value) -> Result<Edit<Metadata<'a>>> {
    metadata_keys(args, &["id", "patch", "expected"])?;
    metadata_id(&args["id"])?;
    Edit::new(repo, &args["patch"], &args["expected"])
}
fn metadata_position(state: &Value, id: &str) -> Option<usize> {
    state["artifacts"]
        .as_array()?
        .iter()
        .position(|entry| entry["id"] == id)
}
fn metadata_apply(
    repo: &Repository,
    state: &mut Value,
    index: usize,
    edit: &Edit<Metadata>,
    discovery: Option<&str>,
) -> Result<Value> {
    metadata_view(repo, &state["artifacts"][index], discovery)?;
    let current = Metadata::new(&state["artifacts"][index], discovery)?;
    let changed = edit.apply(&current, |field, value| {
        match field {
            "title" if value == "" => return Ok(false),
            "sessionIds" => {
                for id in arr(value) {
                    if !arr(&current.values["sessionIds"]).contains(&id)
                        && sessions::get(state, id.as_str().unwrap()).is_err()
                    {
                        return Err(Error::new(
                            "Choose existing sessions; retained older associations may stay unchanged.",
                        ));
                    }
                }
            }
            _ => {}
        }
        if current.values[field] == *value {
            return Ok(false);
        }
        state["artifacts"][index][field] = value.clone();
        Ok(true)
    })?;
    let mut result = metadata_view(repo, &state["artifacts"][index], None)?;
    result["saved"] = json!(true);
    result["changedFields"] = json!(changed);
    Ok(result)
}
fn discovery_basis(state: &Value) -> Value {
    json!({"skillPresets":state["skillPresets"],"artifacts":state["artifacts"],"sessions":sessions::all(state).iter().map(|row|json!({"id":row["id"],"cwd":row["cwd"]})).collect::<Vec<_>>()})
}
fn discovery_config_hash(repo: &Repository) -> Result<String> {
    Ok(super::skill_presets::load(repo)?.hash)
}
fn discovered_entry(repo: &Repository, basis: &Value, id: &str) -> Result<(Value, String, String)> {
    let config = discovery_config_hash(repo)?;
    let entry = arr(&catalog(repo, basis)?["artifacts"])
        .into_iter()
        .find(|entry| entry["id"] == id && entry["discovered"] == true)
        .ok_or_else(|| field_edit::conflict(METADATA_CHANGED))?;
    if discovery_config_hash(repo)? != config {
        return Err(field_edit::conflict(METADATA_CHANGED));
    }
    let fingerprint = context_kit::sha256(&serde_json::to_vec(
        &json!({"entry":entry,"configRevision":config}),
    )?);
    Ok((entry, fingerprint, config))
}
pub fn metadata_read(repo: &Repository, args: &Value) -> Result<Value> {
    metadata_keys(args, &["id"])?;
    let id = metadata_id(&args["id"])?;
    field_edit::exact(repo, id)?;
    let registered = {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        metadata_position(&state, id)
            .map(|index| state["artifacts"][index].clone())
            .ok_or_else(|| discovery_basis(&state))
    };
    match registered {
        Ok(entry) => metadata_view(repo, &entry, None),
        Err(basis) => {
            let (entry, fingerprint, _) = discovered_entry(repo, &basis, id)?;
            metadata_view(repo, &entry, Some(&fingerprint))
        }
    }
}
pub fn metadata_update(repo: &Repository, args: &Value) -> Result<Value> {
    let edit = metadata_patch(repo, args)?;
    let id = metadata_id(&args["id"])?;
    field_edit::exact(repo, id)?;
    repo.transaction_if_changed(|state| {
        let index =
            metadata_position(state, id).ok_or_else(|| field_edit::conflict(METADATA_CHANGED))?;
        metadata_apply(repo, state, index, &edit, None)
    })
}
pub fn register_discovered(repo: &Repository, args: &Value) -> Result<Value> {
    let edit = metadata_patch(repo, args)?;
    let id = metadata_id(&args["id"])?;
    field_edit::exact(repo, id)?;
    let basis = {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        if metadata_position(&state, id).is_some() {
            return Err(field_edit::conflict(METADATA_CHANGED));
        }
        discovery_basis(&state)
    };
    let (entry, fingerprint, _) = discovered_entry(repo, &basis, id)?;
    metadata_view(repo, &entry, Some(&fingerprint))?;
    repo.transaction(|state| {
        if discovery_basis(state) != basis || metadata_position(state, id).is_some() {
            return Err(field_edit::conflict(METADATA_CHANGED));
        }
        let mut stored = json!({});
        for field in [
            "id",
            "title",
            "path",
            "presetId",
            "role",
            "sessionIds",
            "at",
            "format",
        ] {
            if let Some(value) = entry.get(field) {
                stored[field] = value.clone();
            }
        }
        state["artifacts"]
            .as_array_mut()
            .ok_or_else(|| Error::new("Artifact storage is unavailable."))?
            .push(stored);
        let index = state["artifacts"].as_array().unwrap().len() - 1;
        let mut result = metadata_apply(repo, state, index, &edit, Some(&fingerprint))?;
        result["materialized"] = json!(true);
        Ok(result)
    })
}
pub(super) fn legacy_metadata_update(
    repo: &Repository,
    state: &mut Value,
    args: &Value,
) -> Result<Value> {
    metadata_keys(args, &["action", "id", "patch", "expected"])?;
    let mut args = args.clone();
    args.as_object_mut().unwrap().remove("action");
    let edit = metadata_patch(repo, &args)?;
    let id = metadata_id(&args["id"])?;
    let index =
        metadata_position(state, id).ok_or_else(|| field_edit::conflict(METADATA_CHANGED))?;
    metadata_apply(repo, state, index, &edit, None)
}
fn snapshot_entry(repo: &Repository, s: &Value, p: &Value) -> Result<Value> {
    let path = file(text(p, "path"), &roots(repo, s)?)?;
    let bytes = std::fs::read(&path)?;
    if path.extension().is_some_and(|x| x == "json") {
        validate_table(&serde_json::from_slice(&bytes)?)?;
    }
    let extension = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let key = id(&format!(
        "{}:{extension}:{:x}",
        path.to_string_lossy(),
        Sha256::digest(&bytes)
    ));
    let stored = store_bytes(repo, &format!("{key}.{extension}"), &bytes)?;
    Ok(
        json!({"id":key,"path":stored,"sourcePath":path,"at":now(),"format":extension,"snapshot":true}),
    )
}
pub fn save(repo: &Repository, s: &mut Value, p: &Value) -> Result<Value> {
    if p.get("id").is_some() {
        return Err(Error::new("Use artifacts.metadata.read and a captured metadata.update patch for an existing artifact."));
    }
    let ids = p["sessionIds"].as_array().cloned().unwrap_or_default();
    if ids.len() > 100
        || ids
            .iter()
            .any(|id| id.as_str().is_none_or(|id| sessions::get(s, id).is_err()))
    {
        return Err(Error::new("Choose existing sessions."));
    }
    let mut entry = if text(p, "path").is_empty() {
        let url = safe_url(text(p, "url"))?;
        if !url.starts_with("https://") {
            return Err(Error::new("Enter an HTTPS artifact URL."));
        }
        json!({"id":id(&url),"url":url,"at":now(),"format":"link"})
    } else {
        snapshot_entry(repo, s, p)?
    };
    let old = arr(&s["artifacts"])
        .into_iter()
        .find(|x| x["id"] == entry["id"])
        .unwrap_or_else(|| json!({}));
    let existing = old.is_object() && !old.as_object().unwrap().is_empty();
    if existing {
        field_edit::exact(repo, text(&old, "id"))?;
        let supplied_title = checked(p, "title", 300, false)?;
        if (!supplied_title.is_empty() && supplied_title != text(&old, "title"))
            || (!text(p, "presetId").is_empty() && p["presetId"] != old["presetId"])
        {
            let mut error = field_edit::conflict("This artifact already exists. Open its metadata editor to change the title or session links. Existing provenance cannot be reassigned.");
            error.data = json!({"artifactId":old["id"]});
            return Err(error);
        }
        entry = old.clone();
    }
    let title = if text(p, "title").trim().is_empty() {
        old["title"]
            .as_str()
            .or_else(|| entry["title"].as_str())
            .unwrap_or_else(|| {
                if text(&entry, "path").is_empty() {
                    "Artifact"
                } else {
                    "Report"
                }
            })
            .to_string()
    } else {
        checked(p, "title", 300, true)?
    };
    let mut links = if p["replaceSessions"] == true && !existing {
        vec![]
    } else {
        arr(&old["sessionIds"])
    };
    links.extend(ids);
    links.sort_by_key(Value::to_string);
    links.dedup();
    if links.len() > 100 {
        return Err(Error::new(
            "Keep at most 100 complete session links per artifact.",
        ));
    }
    entry["sessionIds"] = json!(links);
    entry["title"] = json!(title);
    if !text(p, "presetId").is_empty() && !existing {
        if !presets(repo, s)?
            .iter()
            .any(|preset| preset["id"] == p["presetId"])
        {
            return Err(Error::new("Choose an existing skill for this output."));
        }
        entry["presetId"] = p["presetId"].clone();
        entry["role"] = json!("skill-output");
    } else {
        // Reattaching an immutable snapshot updates its links/title, not its
        // previously registered ownership.
        for field in ["presetId", "role"] {
            if let Some(value) = old.get(field) {
                entry[field] = value.clone();
            }
        }
    }
    let mut entries = arr(&s["artifacts"]);
    entries.retain(|x| x["id"] != entry["id"]);
    entries.push(entry.clone());
    s["artifacts"] = json!(entries);
    Ok(entry)
}
/// Register a file snapshot or HTTPS link for one existing chat. Both the UI
/// and native helper use this owner so skill attribution is inferred identically.
pub fn attach_to_chat(repo: &Repository, record_id: &str, input: &Value) -> Result<Value> {
    let record_id = sid(record_id)?;
    for key in ["path", "url", "title"] {
        if input.get(key).is_some_and(|value| !value.is_string()) {
            return Err(Error::new(format!("Invalid {key}.")));
        }
    }
    if input
        .get("skillOutput")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err(Error::new("skillOutput must be true or false."));
    }
    let path = checked(input, "path", 4096, false)?;
    let url = checked(input, "url", 4096, false)?;
    let title = checked(input, "title", 300, false)?;
    if path.is_empty() == url.is_empty() {
        return Err(Error::new("Choose exactly one artifact file or HTTPS URL."));
    }
    repo.transaction(|state|{
        let record=sessions::get(state,&record_id)?.clone();
        let mut payload=json!({"path":path,"url":url,"title":title,"sessionIds":[record_id]});
        if input["skillOutput"]==true {
            if text(&record,"newsletterId").is_empty(){return Err(Error::new("This chat has no assigned skill. Attach the artifact without --skill-output."));}
            payload["presetId"]=record["newsletterId"].clone();
        }
        let entry=save(repo,state,&payload)?;
        Ok(json!({"artifactId":entry["id"],"recordId":record_id,"saved":true,"scope":"local",
            "role":if entry["role"]=="skill-output"{"skill-output"}else{"attachment"},
            "presetId":entry["presetId"].as_str().filter(|id|id.len()<=80),"snapshot":entry["snapshot"]==true,"format":entry["format"]}))
    })
}
pub fn read(repo: &Repository, id: &str) -> Result<(Vec<u8>, String)> {
    if id.len() != 24 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::new("Invalid artifact ID."));
    }
    let s = repo.state.lock().unwrap();
    let entry = arr(&catalog(repo, &s)?["artifacts"])
        .into_iter()
        .find(|x| x["id"] == id)
        .ok_or_else(|| Error::new("Artifact unavailable."))?;
    let roots = roots(repo, &s)?;
    drop(s);
    let path = file(text(&entry, "path"), &roots)?;
    let kind = kind(&path).unwrap().into();
    let bytes = std::fs::read(path)?;
    if bytes.len() > 10 * 1024 * 1024 {
        return Err(Error::new("Artifact exceeds 10 MiB."));
    }
    if entry["format"] == "json" {
        validate_table(&serde_json::from_slice(&bytes)?)?;
    }
    Ok((bytes, kind))
}
pub fn validate_table(value: &Value) -> Result<()> {
    let columns = arr(&value["columns"]);
    let rows = arr(&value["rows"]);
    if !["truffle.table/v1", "mydash.table/v1"].contains(&value["schema"].as_str().unwrap_or(""))
        || columns.is_empty()
        || columns.len() > 100
        || columns.iter().any(|x| !x.is_string())
        || !value["rows"].is_array()
        || rows.len() > 10000
        || rows.iter().any(|x| {
            !x.is_array()
                || arr(x).len() != columns.len()
                || arr(x)
                    .iter()
                    .any(|cell| cell.is_object() || cell.is_array())
        })
    {
        return Err(Error::new(
            "Use truffle.table/v1 JSON with columns and up to 10,000 equal-width rows.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod attachment_tests;

#[cfg(test)]
mod metadata_tests;
