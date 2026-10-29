//! Local project metadata. No launch, provider, shell, or trust-setting effects.
use super::{
    arr, checked, directory, domain, json, now, safe_url, text, uuid, Error, Repository, Result,
    Value,
};
const FIELDS: [&str; 5] = ["title", "detail", "url", "cwd", "status"];
fn object_keys(value: &Value, allowed: &[&str]) -> Result<()> {
    let fields = value
        .as_object()
        .ok_or(Error::new("Project fields must be an object."))?;
    if fields.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(Error::new("Unknown project field."));
    }
    Ok(())
}
fn project_id(value: &Value) -> Result<&str> {
    value
        .as_str()
        .filter(|id| {
            !id.trim().is_empty() && id.chars().count() <= 160 && !id.chars().any(char::is_control)
        })
        .ok_or(Error::new("Supply the exact local project ID."))
}
fn position(state: &Value, id: &str) -> Result<usize> {
    arr(&state["notes"])
        .iter()
        .position(|row| row["id"] == id && row["source"] == "notes")
        .ok_or(Error::new("Local project no longer exists."))
}
fn field_token(state: &Value, project: &Value, field: &str) -> Result<String> {
    let value = if field == "status" {
        json!({"status":project["status"],"override":state["overrides"][text(project,"id")]["status"]})
    } else {
        project[field].clone()
    };
    Ok(context_kit::sha256(&serde_json::to_vec(&value)?))
}
fn view(state: &Value, index: usize) -> Result<Value> {
    let row = &state["notes"][index];
    let id = text(row, "id");
    let mut project = json!({"id":id,"source":"notes","kind":"Project"});
    for (field, bound) in [
        ("title", 300),
        ("detail", 10000),
        ("url", 4096),
        ("cwd", 4096),
    ] {
        let value = row[field].as_str().unwrap_or("");
        if value.chars().count() > bound {
            return Err(Error::new(
                "Stored project metadata exceeds its editable limit.",
            ));
        }
        project[field] = json!(value);
    }
    let status = state["overrides"][id]["status"]
        .as_str()
        .filter(|status| !status.is_empty())
        .or(row["status"].as_str())
        .unwrap_or("todo");
    if status.chars().count() > 32 {
        return Err(Error::new(
            "Stored project status exceeds its editable limit.",
        ));
    }
    project["status"] = json!(status);
    for field in ["updatedAt", "archivedAt"] {
        project[field] = row[field]
            .as_f64()
            .filter(|time| time.is_finite() && *time >= 0.0)
            .map_or(Value::Null, |time| json!(time));
    }
    let mut expected = json!({});
    for field in FIELDS {
        expected[field] = json!(field_token(state, row, field)?);
    }
    Ok(json!({"project":project,"expected":expected,"scope":"local"}))
}
fn normalize(fields: &Value, create: bool) -> Result<Value> {
    object_keys(fields, &FIELDS)?;
    let mut result = json!({});
    for field in FIELDS {
        let Some(value) = fields.get(field) else {
            continue;
        };
        if !value.is_string() {
            return Err(Error::new(format!("Invalid project {field}.")));
        }
        let limit = match field {
            "title" => 300,
            "detail" => 10000,
            "status" => 32,
            _ => 4096,
        };
        let value = checked(fields, field, limit, field == "title")?;
        result[field] = json!(match field {
            "url" => safe_url(&value)?,
            "cwd" if !value.is_empty() => directory(&value)?,
            "status" => {
                if !domain::STATUSES.contains(&value.as_str()) {
                    return Err(Error::new("Invalid project status."));
                }
                value
            }
            _ => value,
        });
    }
    if create {
        if result.get("title").is_none() {
            return Err(Error::new("Supply a project title."));
        }
        for field in ["detail", "url", "cwd"] {
            if result.get(field).is_none() {
                result[field] = json!("");
            }
        }
        if result.get("status").is_none() {
            result["status"] = json!("todo");
        }
    } else if result.as_object().unwrap().is_empty() {
        return Err(Error::new("Supply at least one project change."));
    }
    Ok(result)
}
fn expected_keys(patch: &Value, expected: &Value) -> Result<()> {
    let expected = expected.as_object().ok_or(Error::new(
        "Supply captured tokens for exactly the changed project fields.",
    ))?;
    if expected.len() != patch.as_object().unwrap().len()
        || patch
            .as_object()
            .unwrap()
            .keys()
            .any(|key| !expected.contains_key(key))
        || expected.values().any(|value| {
            !value.as_str().is_some_and(|hash| {
                hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        })
    {
        return Err(Error::new(
            "Supply captured tokens for exactly the changed project fields.",
        ));
    }
    Ok(())
}
fn create_normalized(state: &mut Value, fields: &Value) -> Result<Value> {
    let mut row = fields.clone();
    row["id"] = json!(format!("note:{}", uuid()));
    row["source"] = json!("notes");
    row["kind"] = json!("Project");
    row["project"] = json!("Personal");
    row["reference"] = json!("");
    row["sourceStatus"] = json!("");
    row["archivedAt"] = Value::Null;
    row["updatedAt"] = json!(now());
    let rows = state["notes"]
        .as_array_mut()
        .ok_or(Error::new("Invalid local project storage."))?;
    rows.push(row);
    let index = rows.len() - 1;
    let mut result = view(state, index)?;
    result["saved"] = json!(true);
    result["created"] = json!(true);
    result["changedFields"] = json!(FIELDS);
    Ok(result)
}
fn update_normalized(
    state: &mut Value,
    id: &str,
    patch: &Value,
    expected: &Value,
) -> Result<Value> {
    expected_keys(patch, expected)?;
    let index = position(state, id)?;
    for field in patch.as_object().unwrap().keys() {
        if expected[field] != field_token(state, &state["notes"][index], field)? {
            let mut error=Error::new("This project field changed. Read the project again and reapply only your intended changes.");
            error.status = 409;
            return Err(error);
        }
    }
    let mut changed = vec![];
    for (field, value) in patch.as_object().unwrap() {
        let cleared_override = field == "status" && state["overrides"][id].get("status").is_some();
        if state["notes"][index][field] != *value || cleared_override {
            changed.push(field.clone());
            state["notes"][index][field] = value.clone();
        }
        if field == "status" {
            if let Some(overrides) = state["overrides"][id].as_object_mut() {
                overrides.remove("status");
            }
        }
    }
    if !changed.is_empty() {
        state["notes"][index]["updatedAt"] = json!(now());
    }
    let mut result = view(state, index)?;
    result["saved"] = json!(true);
    result["changedFields"] = json!(changed);
    Ok(result)
}
pub fn read(repo: &Repository, args: &Value) -> Result<Value> {
    object_keys(args, &["id"])?;
    let id = project_id(&args["id"])?;
    let state = repo.state.lock().map_err(|_| Error::io())?;
    view(&state, position(&state, id)?)
}
pub fn folder(repo: &Repository, args: &Value) -> Result<Value> {
    folder_with(repo, args, &super::workflows::work_root())
}
fn folder_with(repo: &Repository, args: &Value, root: &std::path::Path) -> Result<Value> {
    object_keys(args, &["id"])?;
    let id = project_id(&args["id"])?;
    let (project, context) = {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        let project = state["notes"][position(&state, id)?].clone();
        let context = json!({"itemSettings":{id:state["itemSettings"][id]},"folderMappings":state["folderMappings"]});
        (project, context)
    };
    let cwd = super::workflows::automatic_folder(&context, &project, root, &json!([]));
    Ok(json!({"id":id,"requiresSelection":cwd.is_none(),"cwd":cwd}))
}
pub fn create(repo: &Repository, args: &Value) -> Result<Value> {
    let fields = normalize(args, true)?;
    repo.transaction(|state| create_normalized(state, &fields))
}
pub fn update(repo: &Repository, args: &Value) -> Result<Value> {
    object_keys(args, &["id", "patch", "expected"])?;
    let id = project_id(&args["id"])?;
    let patch = normalize(&args["patch"], false)?;
    expected_keys(&patch, &args["expected"])?;
    repo.transaction_if_changed(|state| update_normalized(state, id, &patch, &args["expected"]))
}
/// Compatibility transport only; existing legacy updates must carry the same captured tokens.
/// Creation from workflows uses this same validator and canonical record owner.
pub(super) fn save_in_state(state: &mut Value, args: &Value) -> Result<Value> {
    object_keys(
        args,
        &[
            "action", "id", "title", "detail", "url", "cwd", "status", "patch", "expected",
        ],
    )?;
    if args.get("id").is_some_and(|id| !id.is_string()) {
        return Err(Error::new("Invalid project ID."));
    }
    let id = text(args, "id");
    let mut fields = json!({});
    for field in FIELDS {
        if let Some(value) = args.get(field) {
            if id.is_empty() && value.is_null() && field != "title" {
                continue;
            }
            fields[field] = value.clone();
        }
    }
    if id.is_empty() {
        if args.get("patch").is_some() || args.get("expected").is_some() {
            return Err(Error::new(
                "New projects accept fields, not an update patch or captured tokens.",
            ));
        }
        let fields = normalize(&fields, true)?;
        return create_normalized(state, &fields);
    }
    project_id(&args["id"])?;
    let patch = if let Some(patch) = args.get("patch") {
        if !fields.as_object().unwrap().is_empty() {
            return Err(Error::new(
                "Choose either a project patch or flat project fields.",
            ));
        }
        patch
    } else {
        &fields
    };
    let patch = normalize(patch, false)?;
    update_normalized(state, id, &patch, &args["expected"])
}

#[cfg(test)]
mod tests;
