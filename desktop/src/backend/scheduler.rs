//! Persisted skill schedules. Claim a due occurrence before launching it; never replay a backlog.
use super::{
    arr, checked, directory, expand, json, now, sessions, text, uuid, Error, Repository, Result,
    Value,
};
use super::{artifacts, workflows};
use std::time::Duration;
pub fn next_due(schedule: &Value, after: f64) -> Result<f64> {
    match text(schedule, "mode") {
        "interval" => {
            let minutes = schedule["everyMinutes"]
                .as_u64()
                .filter(|n| (5..=10080).contains(n))
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| Error::new("Use an interval of 5 minutes to 7 days."))?;
            Ok(f64::from(minutes).mul_add(60.0, after))
        }
        "daily" => {
            let raw = text(schedule, "time");
            let parts = raw
                .split(':')
                .map(str::parse::<i32>)
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| Error::new("Use HH:MM for the daily time."))?;
            if parts.len() != 2 || !(0..24).contains(&parts[0]) || !(0..60).contains(&parts[1]) {
                return Err(Error::new("Use a valid daily time."));
            }
            let seconds = Duration::try_from_secs_f64(after)
                .ok()
                .and_then(|elapsed| libc::time_t::try_from(elapsed.as_secs()).ok())
                .ok_or_else(|| Error::new("Invalid schedule time."))?;
            let mut local: libc::tm = unsafe { std::mem::zeroed() };
            unsafe { libc::localtime_r(&raw const seconds, &raw mut local) };
            local.tm_hour = parts[0];
            local.tm_min = parts[1];
            local.tm_sec = 0;
            local.tm_isdst = -1;
            let mut result = epoch_seconds(unsafe { libc::mktime(&raw mut local) });
            if result <= after {
                local.tm_mday += 1;
                local.tm_isdst = -1;
                result = epoch_seconds(unsafe { libc::mktime(&raw mut local) });
            }
            Ok(result)
        }
        _ => Err(Error::new("Choose daily or interval.")),
    }
}
fn epoch_seconds(time: libc::time_t) -> f64 {
    let magnitude = Duration::from_secs(time.unsigned_abs()).as_secs_f64();
    if time < 0 {
        -magnitude
    } else {
        magnitude
    }
}
const EDITABLE_PRESET: [&str; 6] = ["title", "cwd", "skill", "command", "agent", "description"];
const SCHEDULE_FIELDS: [&str; 5] = ["mode", "time", "everyMinutes", "missed", "enabled"];
use super::skill_presets::{self, Source as PresetSource};
fn editor_error(message: &str, status: u16) -> Error {
    let mut error = Error::new(message);
    error.status = status;
    error
}
fn editor_conflict() -> Error {
    editor_error(
        "This skill configuration changed. Read it again and reapply only your intended changes.",
        409,
    )
}
fn editor_keys(value: &Value, allowed: &[&str]) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| Error::new("Skill editor input must be an object."))?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(Error::new("Unknown skill editor field."));
    }
    if serde_json::to_vec(value)?.len() > 16 * 1024 {
        return Err(Error::new("Skill editor input exceeds 16 KiB."));
    }
    Ok(())
}
fn editor_exact(repo: &Repository, value: &str) -> Result<()> {
    if value.chars().any(char::is_control) {
        return Err(Error::new("This skill identity or edit token is invalid."));
    }
    let mut projected = value.to_owned();
    super::commands::redact_text(repo, &mut projected);
    if value != projected {
        return Err(Error::new(
            "This skill identity or edit token cannot be safely projected.",
        ));
    }
    Ok(())
}
fn source_current(repo: &Repository, source: &PresetSource) -> Result<()> {
    if !source.is_current(repo).unwrap_or(false) {
        return Err(editor_conflict());
    }
    Ok(())
}
fn preset_field_token(
    source: &PresetSource,
    row: &Value,
    custom: bool,
    field: &str,
) -> Result<String> {
    Ok(context_kit::sha256(&serde_json::to_vec(
        &json!({"id":row["id"],"field":field,"value":row[field],"origin":if custom{"custom"}else{"configured"},"source":if custom{Value::Null}else{json!(source.hash)}}),
    )?))
}
fn execution_identity(row: &Value) -> Value {
    json!({"id":row["id"],"cwd":row["cwd"],"skill":row["skill"],"command":row["command"],"agent":row["agent"],"reports":row["reports"]})
}
fn schedule_field_token(row: &Value, schedule: &Value, field: &str) -> Result<String> {
    Ok(context_kit::sha256(&serde_json::to_vec(
        &json!({"preset":execution_identity(row),"exists":schedule.is_object(),"field":field,"value":schedule[field]}),
    )?))
}
fn schedule_view(raw: &Value) -> Value {
    json!({"mode":raw["mode"].as_str().unwrap_or("daily"),"time":raw["time"].as_str().unwrap_or("09:00"),"everyMinutes":raw["everyMinutes"].as_u64().unwrap_or(1440),"missed":raw["missed"].as_str().unwrap_or("latest"),"enabled":raw["enabled"]==true,"nextRun":raw["nextRun"].as_f64().filter(|value|value.is_finite()&&*value>=0.0)})
}
fn editor_view(repo: &Repository, source: &PresetSource, state: &Value, id: &str) -> Result<Value> {
    editor_exact(repo, id)?;
    let (row, custom) = source.find(state, id)?;
    let mut preset = json!({"id":id});
    let mut expected = json!({});
    for field in EDITABLE_PRESET {
        let value = if field == "agent" {
            row[field].as_str().unwrap_or("claude")
        } else {
            row[field].as_str().unwrap_or("")
        };
        let max = match field {
            "description" => 1000,
            "cwd" => 4096,
            "skill" => 500,
            _ => 180,
        };
        if value.chars().count() > max
            || value
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err(Error::new("A saved skill field exceeds the editor limits."));
        }
        preset[field] = json!(value);
        let token = preset_field_token(source, &row, custom, field)?;
        editor_exact(repo, &token)?;
        expected[field] = json!(token);
    }
    let raw = &state["skillSchedules"][id];
    let mut tokens = json!({});
    for field in SCHEDULE_FIELDS {
        let token = schedule_field_token(&row, raw, field)?;
        editor_exact(repo, &token)?;
        tokens[field] = json!(token);
    }
    for value in [
        "custom",
        "configured",
        "daily",
        "interval",
        "latest",
        "skip",
        "claude",
        "codex",
    ] {
        editor_exact(repo, value)?;
    }
    Ok(
        json!({"presetId":id,"preset":preset,"expected":expected,"origin":{"type":if custom{"custom"}else{"configured"}},"schedule":schedule_view(raw),"scheduleExpected":tokens}),
    )
}
fn checked_patch(patch: &Value, expected: &Value, fields: &[&str]) -> Result<()> {
    editor_keys(patch, fields)?;
    let patch = patch.as_object().unwrap();
    let expected = expected.as_object().ok_or_else(|| {
        Error::new("Supply captured tokens for exactly the changed skill fields.")
    })?;
    if patch.is_empty()
        || expected.len() != patch.len()
        || patch.keys().any(|key| !expected.contains_key(key))
        || expected.values().any(|value| {
            !value.as_str().is_some_and(|value| {
                value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        })
    {
        return Err(Error::new(
            "Supply captured tokens for exactly the changed skill fields.",
        ));
    }
    Ok(())
}
fn preset_patch(value: &Value, create: bool) -> Result<Value> {
    editor_keys(value, &EDITABLE_PRESET)?;
    let mut fields = json!({});
    for (key, value) in value.as_object().unwrap() {
        if !value.is_string() {
            return Err(Error::new("Skill preset fields must be text."));
        }
        let max = match key.as_str() {
            "cwd" => 4096,
            "skill" => 500,
            "description" => 1000,
            _ => 180,
        };
        let text = checked(&json!({key:value}), key, max, key != "description")?;
        fields[key] = json!(match key.as_str() {
            "cwd" => directory(&text)?,
            "agent" => {
                if !["claude", "codex"].contains(&text.as_str()) {
                    return Err(Error::new("Choose Claude or Codex."));
                }
                text
            }
            "command" => {
                if !text.starts_with('/') && !text.starts_with('$') {
                    return Err(Error::new(
                        "Enter a skill command such as /daily or $review.",
                    ));
                }
                text
            }
            "skill" => {
                let path = expand(&text);
                if path
                    .components()
                    .any(|part| part == std::path::Component::ParentDir)
                    || path.is_absolute()
                        && (!path.is_file()
                            || path.extension().and_then(|part| part.to_str()) != Some("md"))
                {
                    return Err(Error::new("Choose an existing absolute Markdown skill file, or a path within the working folder."));
                }
                text
            }
            _ => text,
        });
    }
    if create {
        for field in ["title", "cwd", "skill", "command", "agent"] {
            if fields.get(field).is_none() {
                return Err(Error::new(
                    "Supply the skill name, folder, file, command and agent.",
                ));
            }
        }
        if fields.get("description").is_none() {
            fields["description"] = json!("");
        }
    }
    Ok(fields)
}
fn validate_preset_count(source: &PresetSource, state: &Value) -> Result<()> {
    source.overlay(state)?;
    Ok(())
}
pub fn preset_read(repo: &Repository, args: &Value) -> Result<Value> {
    editor_keys(args, &["presetId"])?;
    let id = checked(args, "presetId", 80, true)?;
    let source = skill_presets::load(repo)?;
    let state = repo.state.lock().map_err(|_| Error::io())?;
    editor_view(repo, &source, &state, &id)
}
pub fn save_preset(repo: &Repository, args: &Value) -> Result<Value> {
    if args.get("id").is_some() {
        return Err(Error::new("Creation does not accept an ID. Use skills.preset.update with captured edit tokens for an existing skill."));
    }
    let fields = preset_patch(args, true)?;
    let source = skill_presets::load(repo)?;
    let id = uuid();
    repo.transaction(|state| {
        source_current(repo, &source)?;
        let mut row = fields.clone();
        row["id"] = json!(id);
        row["reports"] = json!("reports");
        state["skillPresets"]
            .as_array_mut()
            .ok_or_else(Error::io)?
            .push(row);
        validate_preset_count(&source, state)?;
        let mut view = editor_view(repo, &source, state, &id)?;
        view["saved"] = json!(true);
        view["created"] = json!(true);
        view["changedFields"] = json!(EDITABLE_PRESET);
        Ok(view)
    })
}
pub fn update_preset(repo: &Repository, args: &Value) -> Result<Value> {
    let source = skill_presets::load(repo)?;
    update_preset_from_source(repo, args, &source)
}
fn update_preset_from_source(
    repo: &Repository,
    args: &Value,
    source: &PresetSource,
) -> Result<Value> {
    editor_keys(args, &["presetId", "patch", "expected"])?;
    let id = checked(args, "presetId", 80, true)?;
    checked_patch(&args["patch"], &args["expected"], &EDITABLE_PRESET)?;
    let patch = preset_patch(&args["patch"], false)?;
    repo.transaction_if_changed(|state| {
        source_current(repo, source)?;
        let (mut row, custom) = source.find(state, &id)?;
        for field in patch.as_object().unwrap().keys() {
            if args["expected"][field] != preset_field_token(source, &row, custom, field)? {
                return Err(editor_conflict());
            }
        }
        let mut changed = vec![];
        for (field, value) in patch.as_object().unwrap() {
            if row[field] != *value {
                row[field] = value.clone();
                changed.push(field.clone());
            }
        }
        if !changed.is_empty() {
            let rows = state["skillPresets"].as_array_mut().ok_or_else(Error::io)?;
            if let Some(existing) = rows.iter_mut().find(|row| row["id"] == id) {
                *existing = row;
            } else {
                rows.push(row);
            }
            validate_preset_count(source, state)?;
        }
        let mut view = editor_view(repo, source, state, &id)?;
        view["saved"] = json!(true);
        view["changedFields"] = json!(changed);
        Ok(view)
    })
}
fn validate_schedule(schedule: &Value) -> Result<()> {
    if !["daily", "interval"].contains(&text(schedule, "mode"))
        || !["latest", "skip"].contains(&text(schedule, "missed"))
        || !schedule["enabled"].is_boolean()
    {
        return Err(Error::new(
            "Choose a valid schedule frequency, missed-run policy and enabled state.",
        ));
    }
    let time = text(schedule, "time");
    if time.len() != 5
        || time.as_bytes()[2] != b':'
        || time[..2].parse::<u8>().ok().is_none_or(|hour| hour > 23)
        || time[3..]
            .parse::<u8>()
            .ok()
            .is_none_or(|minute| minute > 59)
    {
        return Err(Error::new("Use a valid HH:MM daily time."));
    }
    if !schedule["everyMinutes"]
        .as_u64()
        .is_some_and(|minutes| (5..=10080).contains(&minutes))
    {
        return Err(Error::new("Use an interval of 5 minutes to 7 days."));
    }
    Ok(())
}
pub fn save_schedule(repo: &Repository, args: &Value) -> Result<Value> {
    if args.get("patch").is_none() {
        return Err(Error::new(
            "Reopen the schedule editor to load captured edit tokens before saving.",
        ));
    }
    editor_keys(args, &["presetId", "patch", "expected"])?;
    let id = checked(args, "presetId", 80, true)?;
    checked_patch(&args["patch"], &args["expected"], &SCHEDULE_FIELDS)?;
    let source = skill_presets::load(repo)?;
    repo.transaction_if_changed(|state|{source_current(repo,&source)?;let(row,_)=source.find(state,&id)?;let old=state["skillSchedules"][&id].clone();for field in args["patch"].as_object().unwrap().keys(){if args["expected"][field]!=schedule_field_token(&row,&old,field)?{return Err(editor_conflict());}}
 let old_view=schedule_view(&old);let mut schedule=if old.is_object(){old.clone()}else{old_view.clone()};let mut changed=vec![];
 for(field,value)in args["patch"].as_object().unwrap(){if old_view[field]!=*value||!old.is_object(){changed.push(field.clone());}schedule[field]=value.clone();}
 // Normalize only the known absent defaults; never rebuild the persisted row.
 for field in SCHEDULE_FIELDS{if schedule.get(field).is_none(){schedule[field]=old_view[field].clone();}}
 validate_schedule(&schedule)?;
 if !changed.is_empty(){let cadence_changed=old_view["mode"]!=schedule["mode"]||(schedule["mode"]=="daily"&&old_view["time"]!=schedule["time"])||(schedule["mode"]=="interval"&&old_view["everyMinutes"]!=schedule["everyMinutes"]);if !old.is_object()||schedule["enabled"]==true&&(cadence_changed||old_view["enabled"]!=true){schedule["nextRun"]=json!(next_due(&schedule,now())?);}schedule["presetId"]=json!(id);schedule["updatedAt"]=json!(now());state["skillSchedules"][&id]=schedule;}
 let view=editor_view(repo,&source,state,&id)?;Ok(json!({"saved":true,"presetId":id,"schedule":view["schedule"],"scheduleExpected":view["scheduleExpected"],"changedFields":changed}))})
}
fn list_text(repo: &Repository, value: &Value, max: usize) -> Option<Value> {
    value.as_str().map(|value| {
        let mut value = value.to_owned();
        super::commands::redact_text(repo, &mut value);
        json!(value.chars().take(max).collect::<String>())
    })
}
fn list_identity(repo: &Repository, value: &Value) -> Result<Value> {
    let id = value
        .as_str()
        .filter(|id| !id.is_empty() && id.chars().count() <= 80)
        .ok_or_else(|| Error::new("A saved skill identity is invalid."))?;
    editor_exact(repo, id)?;
    Ok(json!(id))
}
/// Current catalog and its schedules, plus the existing bounded recent-run window.
/// No private preset/schedule/run extensions or executable receipt bodies escape.
pub fn list(repo: &Repository) -> Result<Value> {
    let state = repo.state.lock().map_err(|_| Error::io())?.clone();
    let entries = artifacts::presets(repo, &state)?;
    let mut presets = vec![];
    let mut schedules = json!({});
    for raw in entries {
        let id = list_identity(repo, &raw["id"])?;
        let key = id.as_str().unwrap();
        let mut preset = json!({"id":id});
        for (field, max) in [
            ("title", 180),
            ("cwd", 8192),
            ("skill", 8192),
            ("command", 180),
            ("agent", 80),
            ("reports", 8192),
            ("description", 1000),
            ("unavailableReason", 600),
        ] {
            if ["cwd", "skill", "reports"].contains(&field)
                && raw[field]
                    .as_str()
                    .is_some_and(|value| value.chars().count() > max)
            {
                return Err(Error::new(
                    "A saved skill path exceeds the projection limit.",
                ));
            }
            if let Some(value) = list_text(repo, &raw[field], max) {
                preset[field] = value;
            }
        }
        preset["available"] = json!(raw["available"] == true);
        presets.push(preset);
        let raw = &state["skillSchedules"][key];
        if raw.is_object() {
            let mut schedule = json!({"presetId":id});
            for field in ["mode", "time", "missed"] {
                if let Some(value) = list_text(repo, &raw[field], 80) {
                    schedule[field] = value;
                }
            }
            for field in ["nextRun", "updatedAt"] {
                if let Some(value) = raw[field]
                    .as_f64()
                    .filter(|value| value.is_finite() && *value >= 0.0)
                {
                    schedule[field] = json!(value);
                }
            }
            if let Some(value) = raw["everyMinutes"].as_u64() {
                schedule["everyMinutes"] = json!(value);
            }
            if let Some(value) = raw["enabled"].as_bool() {
                schedule["enabled"] = json!(value);
            }
            schedules[key] = schedule;
        }
    }
    let mut runs = vec![];
    for raw in arr(&state["skillRuns"]).into_iter().take(100) {
        let mut row = json!({});
        for field in ["id", "presetId", "sessionId"] {
            if raw.get(field).is_some_and(|value| !value.is_null()) {
                row[field] = list_identity(repo, &raw[field])?;
            }
        }
        for field in ["at", "dueAt"] {
            if let Some(value) = raw[field]
                .as_f64()
                .filter(|value| value.is_finite() && *value >= 0.0)
            {
                row[field] = json!(value);
            }
        }
        if let Some(value) = raw["scheduled"].as_bool() {
            row["scheduled"] = json!(value);
        }
        for (field, max) in [("outcome", 80), ("message", 600)] {
            if let Some(value) = list_text(repo, &raw[field], max) {
                row[field] = value;
            }
        }
        runs.push(row);
    }
    Ok(json!({"presets":presets,"runs":runs,"schedules":schedules}))
}
fn add_run(s: &mut Value, run: Value) {
    let mut runs = arr(&s["skillRuns"]);
    runs.insert(0, run);
    runs.truncate(100);
    s["skillRuns"] = json!(runs);
}
fn run_receipt(preset: &str, record: &Value, run: Option<&Value>, status: &str) -> Value {
    json!({"presetId":preset,"recordId":record["id"],"runId":run.map(|run|run["id"].clone()),"status":status})
}
pub fn execute(
    repo: &Repository,
    preset: &str,
    scheduled: bool,
    run_id: Option<String>,
) -> Result<Value> {
    let state = repo.state.lock().unwrap().clone();
    if !artifacts::presets(repo, &state)?
        .iter()
        .any(|entry| entry["id"] == preset)
    {
        return Err(Error::new("Choose a current skill preset."));
    }
    let inventory = repo.terminals.inventory()?;
    repo.cooldown(&format!("skill-run:{preset}"), 30.0)?;
    // Do not overlap a previous skill run while its agent terminal remains alive.
    if let Some(existing) = sessions::latest(&state, |x| {
        let waiting = x["runtime"]["state"] == "waiting"
            && x["runtime"]["subagents"].as_u64().unwrap_or(0) == 0;
        x["newsletterId"] == preset
            && x["archivedAt"].is_null()
            && !waiting
            && inventory.live(x).is_some()
    }) {
        if !scheduled {
            let mut record = existing.clone();
            let runs = arr(&state["skillRuns"]);
            let previous = runs
                .iter()
                .find(|run| run["presetId"] == preset && run["sessionId"] == record["id"]);
            record["runReceipt"] = run_receipt(preset, &record, previous, "reused");
            return Ok(record);
        }
        let run = run_id.unwrap_or_else(uuid);
        repo.transaction(|s| {
            if let Some(row) = s["skillRuns"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|x| x["id"] == run)
            {
                row["outcome"] = json!("skipped");
                row["message"] = json!("Previous run is still active.");
            }
            Ok(())
        })?;
        return Ok(existing.clone());
    }
    let mut record = super::chat_preparation::save(
        repo,
        super::chat_preparation::Kind::Skill,
        &json!({"presetId":preset}),
    )?;
    let run = run_id.unwrap_or_else(uuid);
    repo.transaction(|s|{
        let entry=json!({"id":run,"presetId":preset,"sessionId":record["id"],"at":now(),"outcome":"starting","scheduled":scheduled});
        if let Some(row)=s["skillRuns"].as_array_mut().unwrap().iter_mut().find(|x|x["id"]==run){*row=entry;}else{add_run(s,entry);}
        Ok(())
    })?;
    let result = workflows::open(repo, text(&record, "id"));
    let run_saved=repo.transaction(|s| {
        if let Some(row) = s["skillRuns"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|x| x["id"] == run)
        {
            match &result {
                Ok(_) if row["outcome"] == "completed" => {}
                Ok(value) => {
                    let status=match text(value,"status"){"accepted"=>"launched","preview"=>"preview",_=>"unknown"};
                    row["outcome"]=json!(status);
                    row["message"]=json!(match status{"preview"=>"Preview only. No agent or terminal was opened.","unknown"=>"The open outcome is unknown. Inspect this saved chat before another open request.",_ if value["localSaved"]!=true=>"The open request was acknowledged, but local metadata was not saved. Refresh the same chat; do not relaunch to repair it.",_=>"The terminal or desktop owner acknowledged the open request. This does not confirm agent readiness or completion."});
                }
                Err(e) => {row["outcome"]=json!("failed");row["message"]=json!(e.message);}
            }
        }
        Ok(())
    }).is_ok();
    let status = match &result {
        Ok(value) => match text(value, "status") {
            "accepted" => "launched",
            "preview" => "preview",
            _ => "unknown",
        },
        Err(error) => {
            let mut error = error.clone();
            error.data["runReceipt"] =
                run_receipt(preset, &record, Some(&json!({"id":run})), "failed");
            return Err(error);
        }
    };
    record["runReceipt"] = run_receipt(preset, &record, Some(&json!({"id":run})), status);
    if let Ok(value) = result {
        record["runReceipt"]["openReceipt"] = public_open_receipt(&value);
    }
    record["runReceipt"]["runSaved"] = json!(run_saved);
    Ok(record)
}
// The launcher response is not a general-purpose public record projection.
fn public_open_receipt(value: &Value) -> Value {
    let mut receipt = json!({});
    for key in ["status", "recordId", "target", "message"] {
        if let Some(value) = value[key].as_str() {
            receipt[key] = json!(value
                .chars()
                .take(if key == "message" { 600 } else { 80 })
                .collect::<String>());
        }
    }
    if let Some(saved) = value["localSaved"].as_bool() {
        receipt["localSaved"] = json!(saved);
    }
    if let Some(state) = value["terminal"]["state"].as_str().filter(|state| {
        [
            "acknowledged",
            "not-applicable",
            "not-started",
            "unconfirmed",
        ]
        .contains(state)
    }) {
        receipt["terminal"] = json!({"state":state});
    }
    receipt
}
pub fn claim(s: &mut Value, time: f64) -> Result<Vec<(String, String)>> {
    let mut due = vec![];
    for (id, value) in s["skillSchedules"].as_object().unwrap().clone() {
        if value["enabled"] != true || value["nextRun"].as_f64().unwrap_or(f64::INFINITY) > time {
            continue;
        }
        let at = value["nextRun"].as_f64().unwrap_or(time);
        s["skillSchedules"][&id]["nextRun"] = json!(next_due(&value, time)?);
        let skipped = value["missed"] == "skip" && time - at > 120.0;
        let run = uuid();
        add_run(
            s,
            json!({"id":run,"presetId":id,"at":time,"dueAt":at,"scheduled":true,"outcome":if skipped{"skipped"}else{"claimed"},"message":if skipped{"Missed while the service was unavailable."}else{"Reserved for launch."}}),
        );
        if !skipped {
            due.push((id, run));
        }
    }
    Ok(due)
}
pub fn tick(repo: &Repository) -> Result<()> {
    if repo.demo {
        return Ok(());
    }
    let needed = {
        let s = repo.state.lock().unwrap();
        s["skillSchedules"].as_object().unwrap().values().any(|v| {
            v["enabled"] == true && v["nextRun"].as_f64().unwrap_or(f64::INFINITY) <= now()
        })
    };
    if !needed {
        return Ok(());
    }
    let due = repo.transaction(|s| claim(s, now()))?;
    for (id, run) in due {
        if let Err(error) = execute(repo, &id, true, Some(run.clone())) {
            let _ = repo.transaction(|s| {
                if let Some(row) = s["skillRuns"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|x| x["id"] == run)
                {
                    row["outcome"] = json!("failed");
                    row["message"] = json!(error.message);
                }
                Ok(())
            });
            let _=repo.log(&json!({"time":super::repository::iso_time(),"source":"skills","operation":"scheduled-run","outcome":"failure"}));
        }
    }
    Ok(())
}

#[cfg(test)]
mod command_receipt_tests;

#[cfg(test)]
mod skills_editor_tests;
