use super::document::{Document, Format, Version};
use super::{json, Error, Repository, Result, Value};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const MAX_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedViews {
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
pub enum Page {
    All,
    Github,
    Linear,
    Sentry,
    Notes,
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Lifecycle {
    Active,
    Completed,
    Any,
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupBy {
    Status,
    Source,
    None,
    Project,
    Stack,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum Predicate {
    All {
        conditions: Vec<Self>,
    },
    Any {
        conditions: Vec<Self>,
    },
    In {
        field: ScalarField,
        values: Vec<String>,
    },
    NotIn {
        field: ScalarField,
        values: Vec<String>,
    },
    ContainsAny {
        field: SetField,
        values: Vec<String>,
    },
    ContainsAll {
        field: SetField,
        values: Vec<String>,
    },
    ContainsNone {
        field: SetField,
        values: Vec<String>,
    },
    Equals {
        field: BoolField,
        value: bool,
    },
    WithinLast {
        field: DateField,
        days: u32,
    },
    OlderThan {
        field: DateField,
        days: u32,
    },
    BeforeDate {
        field: DateField,
        date: String,
    },
    OnOrAfterDate {
        field: DateField,
        date: String,
    },
    CalendarPeriod {
        field: DateField,
        period: CalendarPeriod,
        timezone: String,
    },
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScalarField {
    Source,
    Kind,
    Status,
    SourceStatus,
    Scope,
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SetField {
    Relations,
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BoolField {
    Pinned,
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DateField {
    CreatedAt,
    UpdatedAt,
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CalendarPeriod {
    Today,
    Yesterday,
    SinceYesterday,
    ThisWeek,
    LastWeek,
}

fn date_boundary(date: &str) -> Option<f64> {
    if date.len() != 10
        || !date.bytes().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
    {
        return None;
    }
    let day = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    Some(exact_f64(day.and_hms_opt(0, 0, 0)?.and_utc().timestamp()))
}
// Same value as an `as f64` cast: the high half scales exactly, so the sum rounds once.
fn exact_f64(value: i64) -> f64 {
    let high = i32::try_from(value >> 32).expect("an i64 shifted by 32 bits fits in i32");
    let low = u32::try_from(value & 0xFFFF_FFFF).expect("a 32-bit mask fits in u32");
    f64::from(high).mul_add(4_294_967_296.0, f64::from(low))
}
fn item_time(item: &Value, field: DateField) -> Option<f64> {
    let value = &item[match field {
        DateField::CreatedAt => "createdAt",
        DateField::UpdatedAt => "updatedAt",
    }];
    if let Some(value) = value.as_f64() {
        return value.is_finite().then_some(value);
    }
    let date = chrono::DateTime::parse_from_rfc3339(value.as_str()?).ok()?;
    if date.timestamp_subsec_nanos() >= 1_000_000_000 {
        return None;
    }
    Some(exact_f64(date.timestamp_millis()) / 1000.0)
}
fn calendar_date(time: f64, timezone: chrono_tz::Tz) -> Option<chrono::NaiveDate> {
    if !time.is_finite() {
        return None;
    }
    let milliseconds = (time * 1000.0).floor();
    // Duration converts the whole magnitude exactly; out-of-range values are rejected.
    let magnitude = i64::try_from(
        std::time::Duration::try_from_secs_f64(milliseconds.abs())
            .ok()?
            .as_secs(),
    )
    .ok()?;
    Some(
        chrono::DateTime::from_timestamp_millis(if milliseconds < 0.0 {
            -magnitude
        } else {
            magnitude
        })?
        .with_timezone(&timezone)
        .date_naive(),
    )
}
fn matches_calendar_period(period: CalendarPeriod, timezone: &str, at: f64, time: f64) -> bool {
    use chrono::Datelike;
    let Ok(zone) = timezone.parse::<chrono_tz::Tz>() else {
        return false;
    };
    let Some(today) = calendar_date(time, zone) else {
        return false;
    };
    let Some(date) = calendar_date(at, zone) else {
        return false;
    };
    let offset = match period {
        CalendarPeriod::Today => 0,
        CalendarPeriod::Yesterday | CalendarPeriod::SinceYesterday => 1,
        CalendarPeriod::ThisWeek => today.weekday().num_days_from_monday(),
        CalendarPeriod::LastWeek => today.weekday().num_days_from_monday() + 7,
    };
    let days = match period {
        CalendarPeriod::ThisWeek | CalendarPeriod::LastWeek => 7,
        CalendarPeriod::SinceYesterday => 2,
        _ => 1,
    };
    let Some(start) = today.checked_sub_days(chrono::Days::new(u64::from(offset))) else {
        return false;
    };
    let Some(end) = start.checked_add_days(chrono::Days::new(days)) else {
        return false;
    };
    date >= start && date < end
}

fn invalid() -> Error {
    Error::new("Invalid saved views. Check the fields, operators and values.")
}
fn valid_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.contains('\0')
}
fn validate_filter(filter: &Predicate, depth: usize, nodes: &mut usize) -> Result<()> {
    *nodes += 1;
    if depth > 4 || *nodes > 128 {
        return Err(invalid());
    }
    match filter {
        Predicate::All { conditions } | Predicate::Any { conditions } => {
            if conditions.len() > 32 {
                return Err(invalid());
            }
            for condition in conditions {
                validate_filter(condition, depth + 1, nodes)?;
            }
        }
        Predicate::In { values, .. }
        | Predicate::NotIn { values, .. }
        | Predicate::ContainsAny { values, .. }
        | Predicate::ContainsAll { values, .. }
        | Predicate::ContainsNone { values, .. } => {
            if values.is_empty()
                || values.len() > 64
                || values.iter().any(|value| !valid_text(value, 500))
                || values.iter().collect::<BTreeSet<_>>().len() != values.len()
            {
                return Err(invalid());
            }
        }
        Predicate::Equals { .. } => {}
        Predicate::WithinLast { days, .. } | Predicate::OlderThan { days, .. } => {
            if !(1..=3650).contains(days) {
                return Err(invalid());
            }
        }
        Predicate::BeforeDate { date, .. } | Predicate::OnOrAfterDate { date, .. } => {
            if date_boundary(date).is_none() {
                return Err(invalid());
            }
        }
        Predicate::CalendarPeriod { timezone, .. } => {
            if timezone.len() > 100 || timezone.parse::<chrono_tz::Tz>().is_err() {
                return Err(invalid());
            }
        }
    }
    Ok(())
}
pub fn parse_document(value: &Value) -> Result<SavedViews> {
    if serde_json::to_vec(value)?.len() > MAX_BYTES {
        return Err(Error::new("Saved views exceed 64 KiB."));
    }
    let doc: SavedViews = serde_json::from_value(value.clone()).map_err(|_| invalid())?;
    if doc.version != 1 {
        return Err(Error::new("Unsupported saved views version."));
    }
    if doc.views.len() > 64 {
        return Err(Error::new("Save at most 64 views."));
    }
    let mut ids = BTreeSet::new();
    for view in &doc.views {
        if !valid_text(&view.id, 80)
            || !view
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
            || !ids.insert(&view.id)
            || !valid_text(&view.name, 120)
            || view.query.len() > 1000
            || view.query.contains('\0')
            || matches!(view.group_by, GroupBy::Stack) && !matches!(view.parent, Page::Github)
        {
            return Err(invalid());
        }
        validate_filter(&view.filter, 0, &mut 0)?;
    }
    Ok(doc)
}
impl Format for SavedViews {
    const NAME: &'static str = "saved views configuration";
    const MAX_BYTES: usize = MAX_BYTES;
    const CONFLICT: &'static str = "Saved views changed. Reload the views before saving again.";
    fn parse(bytes: &[u8]) -> Result<Self> {
        parse_document(&serde_json::from_slice(bytes).map_err(|_| invalid())?)
    }
    fn revision(&self) -> u64 {
        self.revision
    }
}
fn document(repo: &Repository) -> Document<SavedViews> {
    Document::new(repo.settings.config_directory().join("views.json"))
}
fn response(doc: &SavedViews, version: &Version) -> Result<Value> {
    let mut value = serde_json::to_value(doc)?;
    value["contentHash"] = json!(version.content_hash);
    Ok(value)
}
pub fn load(repo: &Repository) -> Result<SavedViews> {
    Ok(document(repo).read()?.0)
}
pub fn view(repo: &Repository) -> Result<Value> {
    let (doc, version) = document(repo).read()?;
    response(&doc, &version)
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Update {
    revision: u64,
    content_hash: String,
    views: Vec<SavedView>,
}
pub fn update(repo: &Repository, payload: &Value) -> Result<Value> {
    if serde_json::to_vec(payload)?.len() > MAX_BYTES {
        return Err(Error::new("Saved views exceed 64 KiB."));
    }
    let update: Update = serde_json::from_value(payload.clone()).map_err(|_| invalid())?;
    let expected = Version {
        revision: update.revision,
        content_hash: update.content_hash,
    };
    let next = SavedViews {
        version: 1,
        revision: expected.revision,
        views: update.views,
    };
    let (doc, version) = document(repo).replace(&expected, next)?;
    response(&doc, &version)
}
fn scalar(item: &Value, field: ScalarField) -> Option<String> {
    match field {
        ScalarField::Source => item["source"].as_str().map(str::to_owned),
        ScalarField::Kind => item["kind"].as_str().map(str::to_owned),
        ScalarField::Status => item["status"].as_str().map(str::to_owned),
        ScalarField::SourceStatus => item["sourceStatus"].as_str().map(str::to_owned),
        ScalarField::Scope => {
            if item["source"] == "github" {
                if let Some(url) = item["url"]
                    .as_str()
                    .and_then(|url| url.strip_prefix("https://github.com/"))
                {
                    let parts = url.split('/').collect::<Vec<_>>();
                    if parts.len() >= 3 && !parts[0].is_empty() && !parts[1].is_empty() {
                        return Some(format!("{}/{}", parts[0], parts[1]));
                    }
                }
            }
            item["project"]
                .as_str()
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        }
    }
}
fn matches_filter_at(filter: &Predicate, item: &Value, time: f64) -> bool {
    match filter {
        Predicate::All { conditions } => conditions
            .iter()
            .all(|condition| matches_filter_at(condition, item, time)),
        Predicate::Any { conditions } => conditions
            .iter()
            .any(|condition| matches_filter_at(condition, item, time)),
        Predicate::In { field, values } => {
            scalar(item, *field).is_some_and(|value| values.contains(&value))
        }
        Predicate::NotIn { field, values } => {
            scalar(item, *field).is_some_and(|value| !values.contains(&value))
        }
        Predicate::ContainsAny {
            field: SetField::Relations,
            values,
        } => item["relations"].as_array().is_some_and(|items| {
            items.iter().all(Value::is_string)
                && values.iter().any(|value| items.contains(&json!(value)))
        }),
        Predicate::ContainsAll {
            field: SetField::Relations,
            values,
        } => item["relations"].as_array().is_some_and(|items| {
            items.iter().all(Value::is_string)
                && values.iter().all(|value| items.contains(&json!(value)))
        }),
        Predicate::ContainsNone {
            field: SetField::Relations,
            values,
        } => item["relations"].as_array().is_some_and(|items| {
            items.iter().all(Value::is_string)
                && values.iter().all(|value| !items.contains(&json!(value)))
        }),
        Predicate::Equals {
            field: BoolField::Pinned,
            value,
        } => item["pinned"].as_bool() == Some(*value),
        Predicate::WithinLast { field, days } => item_time(item, *field)
            .is_some_and(|at| at >= f64::mul_add(f64::from(*days), -86400.0, time) && at <= time),
        Predicate::OlderThan { field, days } => item_time(item, *field)
            .is_some_and(|at| at < f64::mul_add(f64::from(*days), -86400.0, time)),
        Predicate::BeforeDate { field, date } => item_time(item, *field)
            .zip(date_boundary(date))
            .is_some_and(|(at, boundary)| at < boundary),
        Predicate::OnOrAfterDate { field, date } => item_time(item, *field)
            .zip(date_boundary(date))
            .is_some_and(|(at, boundary)| at >= boundary),
        Predicate::CalendarPeriod {
            field,
            period,
            timezone,
        } => item_time(item, *field)
            .is_some_and(|at| matches_calendar_period(*period, timezone, at, time)),
    }
}
pub fn evaluate_at(view: &SavedView, item: &Value, time: f64) -> bool {
    let page = match view.parent {
        Page::All => None,
        Page::Github => Some("github"),
        Page::Linear => Some("linear"),
        Page::Sentry => Some("sentry"),
        Page::Notes => Some("notes"),
    };
    if page.is_some_and(|page| item["source"] != page) {
        return false;
    }
    let lifecycle = match view.lifecycle {
        Lifecycle::Active => item["status"] != "done",
        Lifecycle::Completed => item["status"] == "done",
        Lifecycle::Any => true,
    };
    lifecycle && matches_filter_at(&view.filter, item, time)
}

#[cfg(test)]
mod tests;
