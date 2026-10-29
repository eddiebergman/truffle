use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path, sync::Mutex};

static WRITE_LOCK: Mutex<()> = Mutex::new(());
const MAX_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Document {
    pub version: u64,
    pub revision: u64,
    pub views: Vec<SavedView>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedView {
    pub id: String,
    pub name: String,
    pub parent: Page,
    pub lifecycle: Lifecycle,
    pub filter: Predicate,
    pub query: String,
    pub group_by: GroupBy,
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Page { All, Github, Linear, Sentry, Notes }
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Lifecycle { Active, Completed, Any }
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupBy { Status, Source, None, Project, Stack }
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum Predicate {
    All { conditions: Vec<Predicate> },
    Any { conditions: Vec<Predicate> },
    In { field: ScalarField, values: Vec<String> },
    NotIn { field: ScalarField, values: Vec<String> },
    ContainsAny { field: SetField, values: Vec<String> },
    ContainsAll { field: SetField, values: Vec<String> },
    ContainsNone { field: SetField, values: Vec<String> },
    Equals { field: BoolField, value: bool },
    WithinLast { field: DateField, days: u32 },
    OlderThan { field: DateField, days: u32 },
    BeforeDate { field: DateField, date: String },
    OnOrAfterDate { field: DateField, date: String },
    CalendarPeriod { field: DateField, period: CalendarPeriod, timezone: String },
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScalarField { Source, Kind, Status, SourceStatus, Scope }
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SetField { Relations }
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BoolField { Pinned }
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DateField { CreatedAt, UpdatedAt }
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CalendarPeriod { Today, Yesterday, SinceYesterday, ThisWeek, LastWeek }

fn date_boundary(date: &str) -> Option<f64> {
    if date.len() != 10 || !date.bytes().enumerate().all(|(i, c)| if i == 4 || i == 7 { c == b'-' } else { c.is_ascii_digit() }) { return None; }
    let day = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    Some(day.and_hms_opt(0, 0, 0)?.and_utc().timestamp() as f64)
}
fn item_time(item: &Value, field: DateField) -> Option<f64> {
    let value = &item[match field { DateField::CreatedAt => "createdAt", DateField::UpdatedAt => "updatedAt" }];
    if let Some(value) = value.as_f64() { return value.is_finite().then_some(value); }
    let date = chrono::DateTime::parse_from_rfc3339(value.as_str()?).ok()?;
    if date.timestamp_subsec_nanos() >= 1_000_000_000 { return None; }
    Some(date.timestamp_millis() as f64 / 1000.0)
}
fn calendar_date(time: f64, timezone: chrono_tz::Tz) -> Option<chrono::NaiveDate> {
    if !time.is_finite() { return None; }
    let milliseconds = time * 1000.0;
    if milliseconds < i64::MIN as f64 || milliseconds >= i64::MAX as f64 { return None; }
    Some(chrono::DateTime::from_timestamp_millis(milliseconds.floor() as i64)?.with_timezone(&timezone).date_naive())
}
fn matches_calendar_period(period: CalendarPeriod, timezone: &str, at: f64, time: f64) -> bool {
    use chrono::Datelike;
    let Ok(zone) = timezone.parse::<chrono_tz::Tz>() else { return false; };
    let Some(today) = calendar_date(time, zone) else { return false; };
    let Some(date) = calendar_date(at, zone) else { return false; };
    let offset = match period { CalendarPeriod::Today => 0, CalendarPeriod::Yesterday | CalendarPeriod::SinceYesterday => 1,
        CalendarPeriod::ThisWeek => today.weekday().num_days_from_monday(),
        CalendarPeriod::LastWeek => today.weekday().num_days_from_monday() + 7 };
    let days = match period { CalendarPeriod::ThisWeek | CalendarPeriod::LastWeek => 7, CalendarPeriod::SinceYesterday => 2, _ => 1 };
    let Some(start) = today.checked_sub_days(chrono::Days::new(u64::from(offset))) else { return false; };
    let Some(end) = start.checked_add_days(chrono::Days::new(days)) else { return false; };
    date >= start && date < end
}

fn invalid() -> Error { Error::new("Invalid saved views. Check the fields, operators and values.") }
fn valid_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.contains('\0')
}
fn validate_filter(filter: &Predicate, depth: usize, nodes: &mut usize) -> Result<()> {
    *nodes += 1;
    if depth > 4 || *nodes > 128 { return Err(invalid()); }
    match filter {
        Predicate::All { conditions } | Predicate::Any { conditions } => {
            if conditions.len() > 32 { return Err(invalid()); }
            for condition in conditions { validate_filter(condition, depth + 1, nodes)?; }
        }
        Predicate::In { values, .. } | Predicate::NotIn { values, .. }
        | Predicate::ContainsAny { values, .. } | Predicate::ContainsAll { values, .. }
        | Predicate::ContainsNone { values, .. } => {
            if values.is_empty() || values.len() > 64
                || values.iter().any(|value| !valid_text(value, 500))
                || values.iter().collect::<BTreeSet<_>>().len() != values.len() {
                return Err(invalid());
            }
        }
        Predicate::Equals { .. } => {}
        Predicate::WithinLast { days, .. } | Predicate::OlderThan { days, .. } => {
            if !(1..=3650).contains(days) { return Err(invalid()); }
        }
        Predicate::BeforeDate { date, .. } | Predicate::OnOrAfterDate { date, .. } => {
            if date_boundary(date).is_none() { return Err(invalid()); }
        }
        Predicate::CalendarPeriod { timezone, .. } => {
            if timezone.len() > 100 || timezone.parse::<chrono_tz::Tz>().is_err() { return Err(invalid()); }
        }
    }
    Ok(())
}
pub fn parse_document(value: &Value) -> Result<Document> {
    if serde_json::to_vec(value)?.len() > MAX_BYTES { return Err(Error::new("Saved views exceed 64 KiB.")); }
    let doc: Document = serde_json::from_value(value.clone()).map_err(|_| invalid())?;
    if doc.revision > 9_007_199_254_740_991 { return Err(invalid()); }
    if doc.version != 1 { return Err(Error::new("Unsupported saved views version.")); }
    if doc.views.len() > 64 { return Err(Error::new("Save at most 64 views.")); }
    let mut ids = BTreeSet::new();
    for view in &doc.views {
        if !valid_text(&view.id, 80) || !view.id.bytes().all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
            || !ids.insert(&view.id) || !valid_text(&view.name, 120)
            || view.query.len() > 1000 || view.query.contains('\0')
            || matches!(view.group_by, GroupBy::Stack) && !matches!(view.parent, Page::Github) {
            return Err(invalid());
        }
        validate_filter(&view.filter, 0, &mut 0)?;
    }
    Ok(doc)
}
fn read_document(path: &Path) -> Result<(Document, String)> {
    let bytes = context_kit::read_bounded(path, MAX_BYTES as u64)
        .map_err(|_| Error::new("Cannot read saved views configuration."))?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    Ok((parse_document(&value)?, format!("{:x}", Sha256::digest(&bytes))))
}
fn response(doc: &Document, hash: String) -> Result<Value> {
    let mut value = serde_json::to_value(doc)?;
    value["contentHash"] = json!(hash);
    Ok(value)
}
pub fn load(repo: &Repository) -> Result<Document> {
    Ok(read_document(&repo.settings.config_directory().join("views.json"))?.0)
}
pub fn view(repo: &Repository) -> Result<Value> {
    let (doc, hash) = read_document(&repo.settings.config_directory().join("views.json"))?;
    response(&doc, hash)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Update {
    revision: u64,
    content_hash: String,
    views: Vec<SavedView>,
}
fn replacement(current: &Document, hash: &str, payload: &Value) -> Result<Document> {
    if serde_json::to_vec(payload)?.len() > MAX_BYTES { return Err(Error::new("Saved views exceed 64 KiB.")); }
    let update: Update = serde_json::from_value(payload.clone()).map_err(|_| invalid())?;
    if update.revision != current.revision || update.content_hash != hash {
        let mut error = Error::new("Saved views changed. Reload the views before saving again.");
        error.status = 409;
        return Err(error);
    }
    let next = Document { version: 1, revision: current.revision.checked_add(1).ok_or_else(invalid)?, views: update.views };
    parse_document(&serde_json::to_value(&next)?)
}
fn update_file(path: &Path, payload: &Value) -> Result<Value> {
    let _guard = WRITE_LOCK.lock().map_err(|_| Error::io())?;
    let (current, hash) = read_document(path)?;
    let next = replacement(&current, &hash, payload)?;
    let bytes = serde_json::to_vec_pretty(&next)?;
    if bytes.len() > MAX_BYTES { return Err(Error::new("Saved views exceed 64 KiB.")); }
    let temp = path.with_extension("json.tmp");
    super::http::private_write(&temp, &bytes)?;
    std::fs::rename(&temp, path)?;
    response(&next, format!("{:x}", Sha256::digest(&bytes)))
}
pub fn update(repo: &Repository, payload: &Value) -> Result<Value> {
    update_file(&repo.settings.config_directory().join("views.json"), payload)
}
fn scalar(item: &Value, field: ScalarField) -> Option<String> {
    match field {
        ScalarField::Source => item["source"].as_str().map(str::to_owned),
        ScalarField::Kind => item["kind"].as_str().map(str::to_owned),
        ScalarField::Status => item["status"].as_str().map(str::to_owned),
        ScalarField::SourceStatus => item["sourceStatus"].as_str().map(str::to_owned),
        ScalarField::Scope => {
            if item["source"] == "github" {
                if let Some(url) = item["url"].as_str().and_then(|url| url.strip_prefix("https://github.com/")) {
                    let parts = url.split('/').collect::<Vec<_>>();
                    if parts.len() >= 3 && !parts[0].is_empty() && !parts[1].is_empty() {
                        return Some(format!("{}/{}", parts[0], parts[1]));
                    }
                }
            }
            item["project"].as_str().filter(|value| !value.is_empty()).map(str::to_owned)
        }
    }
}
pub fn matches_filter(filter: &Predicate, item: &Value) -> bool {
    matches_filter_at(filter, item, now())
}
fn matches_filter_at(filter: &Predicate, item: &Value, time: f64) -> bool {
    match filter {
        Predicate::All { conditions } => conditions.iter().all(|condition| matches_filter_at(condition, item, time)),
        Predicate::Any { conditions } => conditions.iter().any(|condition| matches_filter_at(condition, item, time)),
        Predicate::In { field, values } => scalar(item, *field).is_some_and(|value| values.contains(&value)),
        Predicate::NotIn { field, values } => scalar(item, *field).is_some_and(|value| !values.contains(&value)),
        Predicate::ContainsAny { field: SetField::Relations, values } => item["relations"].as_array().is_some_and(|items| items.iter().all(Value::is_string) && values.iter().any(|value| items.contains(&json!(value)))),
        Predicate::ContainsAll { field: SetField::Relations, values } => item["relations"].as_array().is_some_and(|items| items.iter().all(Value::is_string) && values.iter().all(|value| items.contains(&json!(value)))),
        Predicate::ContainsNone { field: SetField::Relations, values } => item["relations"].as_array().is_some_and(|items| items.iter().all(Value::is_string) && values.iter().all(|value| !items.contains(&json!(value)))),
        Predicate::Equals { field: BoolField::Pinned, value } => item["pinned"].as_bool() == Some(*value),
        Predicate::WithinLast { field, days } => item_time(item, *field).is_some_and(|at| at >= time - f64::from(*days) * 86400.0 && at <= time),
        Predicate::OlderThan { field, days } => item_time(item, *field).is_some_and(|at| at < time - f64::from(*days) * 86400.0),
        Predicate::BeforeDate { field, date } => item_time(item, *field).zip(date_boundary(date)).is_some_and(|(at, boundary)| at < boundary),
        Predicate::OnOrAfterDate { field, date } => item_time(item, *field).zip(date_boundary(date)).is_some_and(|(at, boundary)| at >= boundary),
        Predicate::CalendarPeriod { field, period, timezone } => item_time(item, *field).is_some_and(|at| matches_calendar_period(*period, timezone, at, time)),
    }
}
pub fn evaluate(view: &SavedView, item: &Value) -> bool {
    evaluate_at(view, item, now())
}
pub fn evaluate_at(view: &SavedView, item: &Value, time: f64) -> bool {
    let page = match view.parent { Page::All => None, Page::Github => Some("github"), Page::Linear => Some("linear"), Page::Sentry => Some("sentry"), Page::Notes => Some("notes") };
    if page.is_some_and(|page| item["source"] != page) { return false; }
    let lifecycle = match view.lifecycle { Lifecycle::Active => item["status"] != "done", Lifecycle::Completed => item["status"] == "done", Lifecycle::Any => true };
    lifecycle && matches_filter_at(&view.filter, item, time)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn defaults() -> Value { serde_json::from_str(include_str!("../../../static/config/views.json")).unwrap() }
    #[test]
    fn defaults_and_missing_fields_have_explicit_meanings() {
        let doc = parse_document(&defaults()).unwrap();
        assert!(evaluate(&doc.views[0], &json!({"source":"github","kind":"Pull request","relations":["authored"],"status":"unknown"})));
        assert!(!evaluate(&doc.views[0], &json!({"source":"github","kind":"Pull request","relations":["authored"],"status":"done"})));
        for filter in [json!({"op":"notIn","field":"status","values":["done"]}),json!({"op":"containsNone","field":"relations","values":["reviews"]})] {
            let predicate = serde_json::from_value(filter).unwrap();
            assert!(!matches_filter(&predicate, &json!({})));
        }
        let predicate = serde_json::from_value(json!({"op":"all","conditions":[{"op":"in","field":"source","values":["github"]},{"op":"any","conditions":[{"op":"equals","field":"pinned","value":true},{"op":"containsAny","field":"relations","values":["reviews"]}]}]})).unwrap();
        assert!(matches_filter(&predicate,&json!({"source":"github","relations":["reviews"]})));
        assert!(!matches_filter(&predicate,&json!({"source":"linear","pinned":true})));
    }
    #[test]
    fn validation_rejects_versions_invalid_types_duplicates_and_depth() {
        let mut value = defaults();value["version"] = json!(2);assert!(parse_document(&value).is_err());
        value = defaults();value["views"][1]["id"] = value["views"][0]["id"].clone();assert!(parse_document(&value).is_err());
        value = defaults();value["views"][0]["filter"] = json!({"op":"containsAny","field":"status","values":["done"]});assert!(parse_document(&value).is_err());
        let mut deep = json!({"op":"all","conditions":[]});for _ in 0..5 {deep = json!({"op":"all","conditions":[deep]});}value = defaults();value["views"][0]["filter"] = deep;assert!(parse_document(&value).is_err());
    }
    #[test]
    fn temporal_filters_match_utc_boundaries_and_rolling_windows() {
        let time = date_boundary("2026-10-27").unwrap();
        for source in ["linear", "github"] {
            for field in ["createdAt", "updatedAt"] {
                let within: Predicate = serde_json::from_value(json!({"op":"withinLast","field":field,"days":7})).unwrap();
                let older: Predicate = serde_json::from_value(json!({"op":"olderThan","field":field,"days":7})).unwrap();
                let mut item = json!({"source":source});
                item[field] = json!(time - 7.0 * 86400.0);
                assert!(matches_filter_at(&within, &item, time));
                assert!(!matches_filter_at(&older, &item, time));
                item[field] = json!(time - 7.0 * 86400.0 - 0.001);
                assert!(!matches_filter_at(&within, &item, time));
                assert!(matches_filter_at(&older, &item, time));
                item[field] = json!(time + 1.0);
                assert!(!matches_filter_at(&within, &item, time));
                let before: Predicate = serde_json::from_value(json!({"op":"beforeDate","field":field,"date":"2026-10-27"})).unwrap();
                let after: Predicate = serde_json::from_value(json!({"op":"onOrAfterDate","field":field,"date":"2026-10-27"})).unwrap();
                item[field] = json!("2026-10-27T02:00:00+02:00");
                assert!(!matches_filter_at(&before, &item, time));
                assert!(matches_filter_at(&after, &item, time));
                item[field] = json!("2026-10-26T23:59:59.999Z");
                assert!(matches_filter_at(&before, &item, time));
                assert!(!matches_filter_at(&after, &item, time));
                for bad in [Value::Null, json!(true), json!("yesterday"), json!("2026-10-27"), json!("2026-10-27T00:00:00")] {
                    item[field] = bad;
                    for filter in [&within, &older, &before, &after] { assert!(!matches_filter_at(filter, &item, time)); }
                }
            }
        }
        for filter in [json!({"op":"withinLast","field":"updatedAt","days":0}),json!({"op":"olderThan","field":"createdAt","days":3651}),json!({"op":"withinLast","field":"status","days":7}),json!({"op":"beforeDate","field":"updatedAt","date":"2026-02-30"}),json!({"op":"onOrAfterDate","field":"createdAt","date":"2026-2-01"})] {
            let mut doc = defaults(); doc["views"][0]["filter"] = filter;
            assert!(parse_document(&doc).is_err());
        }
        assert!(date_boundary("2024-02-29").is_some());
    }
    #[test]
    fn calendar_filters_use_saved_timezones_and_calendar_days_across_dst() {
        let timestamp = |value: &str| chrono::DateTime::parse_from_rfc3339(value).unwrap().timestamp() as f64;
        for (time, before, first, last, after) in [
            ("2026-03-30T10:00:00Z", "2026-03-28T22:59:59Z", "2026-03-28T23:00:00Z", "2026-03-29T21:59:59Z", "2026-03-29T22:00:00Z"),
            ("2026-10-26T10:00:00Z", "2026-10-24T21:59:59Z", "2026-10-24T22:00:00Z", "2026-10-25T22:59:59Z", "2026-10-25T23:00:00Z"),
        ] {
            let filter: Predicate = serde_json::from_value(json!({"op":"calendarPeriod","field":"updatedAt","period":"yesterday","timezone":"Europe/Berlin"})).unwrap();
            for (at, expected) in [(before,false),(first,true),(last,true),(after,false)] {
                assert_eq!(matches_filter_at(&filter, &json!({"updatedAt":at}), timestamp(time)), expected, "{at}");
            }
            assert!(!matches_filter_at(&filter, &json!({}), timestamp(time)));
        }
        let time = timestamp("2027-01-01T10:00:00Z");
        for (period, at, expected) in [
            ("today", "2027-01-01T00:00:00Z", true),
            ("yesterday", "2026-12-31T23:00:00Z", true),
            ("sinceYesterday", "2026-12-31T00:00:00Z", true),
            ("sinceYesterday", "2027-01-01T09:00:00Z", true),
            ("sinceYesterday", "2026-12-30T23:59:59Z", false),
            ("thisWeek", "2026-12-28T00:00:00Z", true),
            ("thisWeek", "2026-12-27T23:59:59Z", false),
            ("lastWeek", "2026-12-21T00:00:00Z", true),
            ("lastWeek", "2026-12-28T00:00:00Z", false),
        ] {
            let filter: Predicate = serde_json::from_value(json!({"op":"calendarPeriod","field":"createdAt","period":period,"timezone":"UTC"})).unwrap();
            assert_eq!(matches_filter_at(&filter, &json!({"createdAt":at}), time), expected);
        }
        for filter in [json!({"op":"calendarPeriod","field":"updatedAt","period":"yesterday","timezone":"invalid/zone"}),json!({"op":"calendarPeriod","field":"updatedAt","period":"tomorrow","timezone":"UTC"})] {
            let mut doc = defaults(); doc["views"][0]["filter"] = filter; assert!(parse_document(&doc).is_err());
        }
    }
    #[test]
    fn cas_catches_stale_revisions_and_external_edits() {
        let directory = std::env::temp_dir().join(format!("mydash-views-{}-{}",std::process::id(),uuid::Uuid::new_v4()));std::fs::create_dir_all(&directory).unwrap();let path = directory.join("views.json");
        std::fs::write(&path,serde_json::to_vec(&defaults()).unwrap()).unwrap();let (doc,hash)=read_document(&path).unwrap();
        let payload=json!({"revision":doc.revision,"contentHash":hash,"views":doc.views});
        let next=update_file(&path,&payload).unwrap();assert_eq!(next["revision"],1);assert_eq!(update_file(&path,&payload).unwrap_err().status,409);
        let payload=json!({"revision":next["revision"],"contentHash":next["contentHash"],"views":next["views"]});
        let mut external:Value=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();external["views"][0]["name"]=json!("External edit");std::fs::write(&path,serde_json::to_vec(&external).unwrap()).unwrap();
        assert_eq!(update_file(&path,&payload).unwrap_err().status,409);assert_eq!(read_document(&path).unwrap().0.views[0].name,"External edit");std::fs::remove_dir_all(directory).unwrap();
    }
}
