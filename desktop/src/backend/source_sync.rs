//! Source synchronization: admission, the forced-request cooldown, retry backoff,
//! and merging provider results into canonical state.
use super::{
    arr, domain, json, now, repository::iso_time, sessions, text, Error, Repository, Result, Value,
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    sync::{Arc, Mutex},
};

pub const SOURCES: [&str; 3] = ["github", "linear", "sentry"];
/// Seconds between routine syncs, and the first retry delay after a failure.
const INTERVAL: f64 = 300.0;
/// GitHub and Linear rate limits reset within an hour, so a longer wait only
/// delays recovery. A longer Retry-After still wins.
const MAX_BACKOFF: f64 = 3600.0;
const FORCE_COOLDOWN: f64 = 15.0;

/// Supplies work items from GitHub, Linear and Sentry.
pub trait ProviderSource: Send + Sync {
    /// The source's current items. For Sentry, optionally one environment's.
    fn fetch(&self, source: &str, environment: Option<&str>) -> Result<Vec<Value>>;
    /// One item per ref. A ref the provider no longer has comes back with
    /// sourceUnavailable set.
    fn refresh(&self, source: &str, refs: &[Value]) -> Result<Vec<Value>>;
}

pub struct SourceSync {
    provider: Arc<dyn ProviderSource>,
    /// Sources with a sync worker. Lock after the repository state.
    pub(super) running: Mutex<BTreeSet<String>>,
    force_after: Mutex<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Skip {
    Disabled,
    AlreadyRunning,
    Backoff(f64),
    Interval(f64),
}
impl Skip {
    const fn reason(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::AlreadyRunning => "already-running",
            Self::Backoff(_) => "backoff",
            Self::Interval(_) => "interval",
        }
    }
    const fn retry_at(self) -> Option<f64> {
        match self {
            Self::Disabled | Self::AlreadyRunning => None,
            Self::Backoff(at) | Self::Interval(at) => Some(at),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    status: &'static str,
    force: bool,
    force_refresh_after: Option<f64>,
    results: Vec<Row>,
    message: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    id: &'static str,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_at: Option<f64>,
}
impl Row {
    const fn started(id: &'static str) -> Self {
        Self {
            id,
            status: "started",
            reason: None,
            retry_at: None,
        }
    }
    const fn skipped(id: &'static str, reason: &'static str, retry_at: Option<f64>) -> Self {
        Self {
            id,
            status: "skipped",
            reason: Some(reason),
            retry_at,
        }
    }
}

impl SourceSync {
    pub fn new(provider: Arc<dyn ProviderSource>) -> Self {
        Self {
            provider,
            running: Mutex::new(BTreeSet::new()),
            force_after: Mutex::new(0.0),
        }
    }
    pub fn syncing(&self) -> Vec<String> {
        self.running.lock().unwrap().iter().cloned().collect()
    }
    /// When the next forced request is allowed.
    pub fn force_after(&self) -> f64 {
        *self.force_after.lock().unwrap()
    }
    /// Why `source` must not start a forced read now, if it must not.
    /// The caller holds the repository state that `info` comes from.
    pub(super) fn forced_skip(
        &self,
        source: &str,
        info: &Value,
        time: f64,
    ) -> Result<Option<Skip>> {
        let running = self.running.lock().map_err(|_| Error::io())?;
        Ok(admission(info, running.contains(source), true, time))
    }
    fn start_force_cooldown(&self) -> Result<()> {
        let mut after = self.force_after.lock().map_err(|_| Error::io())?;
        if *after > now() {
            return Err(Error::new("This action just started. Give it a moment."));
        }
        *after = now() + FORCE_COOLDOWN;
        drop(after);
        Ok(())
    }
}

fn valid_time(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value >= 0.0 && *value <= 9_007_199_254_740_991.0)
}
fn failures(info: &Value) -> u64 {
    info["failures"]
        .as_u64()
        .filter(|value| *value <= 9_007_199_254_740_991)
        .unwrap_or(0)
}
fn admission(info: &Value, running: bool, force: bool, time: f64) -> Option<Skip> {
    if info["disabled"] == true {
        return Some(Skip::Disabled);
    }
    if running {
        return Some(Skip::AlreadyRunning);
    }
    if let Some(next) = valid_time(info["nextAttempt"].as_f64()).filter(|next| *next > time) {
        if info["failures"].as_u64().unwrap_or(0) > 0 {
            return Some(Skip::Backoff(next));
        }
        if !force {
            return Some(Skip::Interval(next));
        }
    }
    None
}
/// Retry delay after `failures` consecutive failures. It doubles from the
/// interval up to the cap, and is never shorter than the provider's Retry-After.
fn backoff(failures: u64, retry_after: f64) -> f64 {
    let doublings = failures.saturating_sub(1).min(4);
    (INTERVAL * f64::from(1_u8 << doublings))
        .min(MAX_BACKOFF)
        .max(retry_after)
}
/// Count one failure in `info` and return the retry delay.
fn record_failure(info: &mut Value, error: &Error) -> f64 {
    let count = info["failures"].as_u64().unwrap_or(0) + 1;
    let retry = backoff(count, error.retry);
    info["failures"] = json!(count);
    info["error"] = json!(error.message);
    info["nextAttempt"] = json!(now() + retry);
    retry
}

/// Start a worker for each admitted source. Returns what started, not what completed.
pub fn request(repo: &Arc<Repository>, force: bool) -> Result<Receipt> {
    request_with(repo, force, |repo, source| {
        std::thread::spawn(move || run(&repo, source));
    })
}
fn request_with(
    repo: &Arc<Repository>,
    force: bool,
    launch: impl Fn(Arc<Repository>, &'static str),
) -> Result<Receipt> {
    if !repo.demo && force {
        repo.sync.start_force_cooldown()?;
        // A no-op force request still changes its cooldown and must invalidate cached snapshots.
        repo.bump_revision();
    }
    let mut results = vec![];
    let mut started = vec![];
    if repo.demo {
        results.extend(SOURCES.map(|id| Row::skipped(id, "demo", None)));
    } else {
        let time = now();
        let mut state = repo.state.lock().map_err(|_| Error::io())?;
        let mut running = repo.sync.running.lock().map_err(|_| Error::io())?;
        for source in SOURCES {
            let info = &mut state["sources"][source];
            if let Some(skip) = admission(info, running.contains(source), force, time) {
                results.push(Row::skipped(source, skip.reason(), skip.retry_at()));
                continue;
            }
            running.insert(source.into());
            info["nextAttempt"] = json!(time + INTERVAL);
            started.push(source);
            results.push(Row::started(source));
        }
        if !started.is_empty() {
            repo.bump_revision();
        }
        drop(running);
        drop(state);
    }
    // Reservations are complete before workers start; never hold state locks across provider I/O.
    for source in started.iter().copied() {
        launch(repo.clone(), source);
    }
    let message = if repo.demo {
        "Sample workspace: no external requests were started.".to_owned()
    } else if started.is_empty() {
        "Nothing started: sources are disabled, already refreshing, or waiting for their retry window.".to_owned()
    } else {
        format!("Refreshing {}…", started.join(" & "))
    };
    Ok(Receipt {
        status: if started.is_empty() {
            "skipped"
        } else {
            "started"
        },
        force,
        force_refresh_after: valid_time(Some(repo.sync.force_after())),
        results,
        message,
    })
}

/// Sync one reserved source, record and log the outcome, and release the reservation.
fn run(repo: &Repository, source: &'static str) {
    let start = now();
    let result = fetch(repo, source).and_then(|(items, refreshed)| {
        repo.transaction(|state| Ok(merge(state, source, items, &refreshed)))
    });
    let mut event = json!({"time":iso_time(),"source":source,"operation":"sync","elapsedMs":((now()-start)*1000.0).round()});
    match result {
        Ok(count) => {
            event["outcome"] = json!("success");
            event["itemCount"] = json!(count);
        }
        Err(error) => {
            let mut retry = INTERVAL;
            let _ = repo.transaction(|state| {
                retry = record_failure(&mut state["sources"][source], &error);
                Ok(())
            });
            event["outcome"] = json!("failure");
            event["retrySeconds"] = json!(retry);
            if let Some(fields) = error.data.as_object() {
                for (key, value) in fields {
                    event[key] = value.clone();
                }
            }
        }
    }
    let _ = repo.log(&event);
    repo.sync.running.lock().unwrap().remove(source);
    repo.bump_revision();
}
/// The provider's listing plus fresh copies of tracked items it omits, and the
/// IDs of those refreshed items.
fn fetch(repo: &Repository, source: &str) -> Result<(Vec<Value>, BTreeSet<String>)> {
    let provider = &repo.sync.provider;
    let mut items = provider.fetch(source, None)?;
    let omitted = omitted(
        &*repo.state.lock().map_err(|_| Error::io())?,
        source,
        &items,
    );
    let refreshed = omitted
        .iter()
        .map(|item| text(item, "id").to_owned())
        .collect();
    if !omitted.is_empty() {
        items.extend(provider.refresh(source, &omitted)?);
    }
    Ok((items, refreshed))
}
/// Tracked items of `source` missing from `fetched`. Items in active chats and
/// hidden items are tracked, and so are open cached GitHub and Linear items.
fn omitted(state: &Value, source: &str, fetched: &[Value]) -> Vec<Value> {
    let catalog = domain::catalog(state);
    let mut ids = BTreeSet::new();
    for session in sessions::all(state) {
        if session["archivedAt"].is_null() {
            for item in arr(&session["items"]) {
                if item["source"] == source {
                    ids.insert(text(&item, "id").to_owned());
                }
            }
        }
    }
    let current = state["items"]
        .as_array()
        .map(|items| items.iter().map(|x| text(x, "id")).collect::<HashSet<_>>())
        .unwrap_or_default();
    for (id, item) in &catalog {
        if item["source"] == source
            && (state["overrides"][id]["hidden"] == true
                || (["github", "linear"].contains(&source)
                    && item["status"] != "done"
                    && current.contains(id.as_str())))
        {
            ids.insert(id.clone());
        }
    }
    let fetched = fetched
        .iter()
        .map(|x| text(x, "id"))
        .collect::<HashSet<_>>();
    ids.into_iter()
        .filter(|id| !fetched.contains(id.as_str()))
        .filter_map(|id| catalog.get(&id).cloned())
        .collect()
}
/// Replace the cached items of `source` with `items` and return the merged count.
/// Hidden items the provider omits stay cached. Refreshed items are also archived.
fn merge(
    state: &mut Value,
    source: &str,
    items: Vec<Value>,
    refreshed: &BTreeSet<String>,
) -> usize {
    let mut merged = items
        .into_iter()
        .map(|item| (text(&item, "id").to_owned(), item))
        .collect::<BTreeMap<_, _>>();
    for item in arr(&state["items"]) {
        let id = text(&item, "id").to_owned();
        if item["source"] == source && state["overrides"][&id]["hidden"] == true {
            merged.entry(id).or_insert(item);
        }
    }
    let items = merged.into_values().collect::<Vec<_>>();
    for session in sessions::all_mut(state) {
        for item in session["items"].as_array_mut().unwrap() {
            if let Some(new) = items.iter().find(|x| x["id"] == item["id"]) {
                *item = domain::item_ref(new);
            }
        }
    }
    for item in &items {
        if refreshed.contains(text(item, "id")) {
            state["itemArchive"][text(item, "id")] = item.clone();
        }
    }
    let mut all = arr(&state["items"]);
    all.retain(|item| item["source"] != source);
    all.extend(items.iter().cloned());
    state["items"] = json!(all);
    domain::observe(state, source, &items);
    if source == "github" {
        super::github::observe(state);
    }
    state["sources"][source] = json!({"lastSuccess":now(),"nextAttempt":now()+INTERVAL,"error":null,"failures":0,"disabled":false});
    items.len()
}

/// Bounded sync metadata. Never expose provider configuration or arbitrary source fields.
pub fn status(repo: &Repository) -> Value {
    let mut sources = {
        let state = repo.state.lock().unwrap();
        SOURCES.into_iter().map(|id| {
            let info = &state["sources"][id];
            json!({"id":id,"syncEnabled":info["disabled"]!=true,
                "lastSuccess":valid_time(info["lastSuccess"].as_f64()),"nextAttempt":valid_time(info["nextAttempt"].as_f64()),
                "failures":failures(info),"hasError":info["error"].is_string(),
                "error":info["error"].as_str(),
                "cachedItemCount":state["items"].as_array().map_or(0,|items|items.iter().filter(|item|item["source"]==id).count())})
        }).collect::<Vec<_>>()
    };
    let running = repo.sync.syncing();
    for row in &mut sources {
        row["refreshing"] = json!(running.iter().any(|id| id == text(row, "id")));
        if let Some(error) = row["error"].as_str() {
            let mut error = error.to_owned();
            super::redaction::redact_text(repo, &mut error);
            row["error"] = json!(error.chars().take(2000).collect::<String>());
        }
    }
    json!({"observedAt":now(),"demo":repo.demo,"forceRefreshAfter":valid_time(Some(repo.sync.force_after())),"sources":sources})
}
/// Cached Sentry environment summaries, and optionally one page of members.
pub fn environments(repo: &Repository, args: &Value) -> Result<Value> {
    let requested = args["environment"].as_str();
    let offset = usize::try_from(args["offset"].as_u64().unwrap_or(0)).unwrap_or(usize::MAX);
    if requested.is_none() && args.get("offset").is_some() {
        return Err(Error::new(
            "An environment is required for membership pagination.",
        ));
    }
    let observed_at = now();
    let mut result = environment_summary(
        &*repo.state.lock().map_err(|_| Error::io())?,
        requested,
        offset,
        observed_at,
        repo.demo,
    )?;
    let original = result.clone();
    super::redaction::redact(repo, &mut result);
    if result != original {
        return Err(Error::new(
            "Environment identifiers cannot be returned safely.",
        ));
    }
    Ok(result)
}
fn environment_summary(
    state: &Value,
    requested: Option<&str>,
    offset: usize,
    observed_at: f64,
    demo: bool,
) -> Result<Value> {
    let caches = state["sentryEnvironments"]
        .as_object()
        .ok_or_else(|| Error::new("Invalid stored environment caches."))?;
    if caches.len() > 20 {
        return Err(Error::new("Too many stored environment caches."));
    }
    let mut summaries = Vec::new();
    let mut selected = requested.map(|name| json!({"environment":name,"cached":false,"membershipKnown":false,"itemIds":[],"total":null,"offset":offset,"nextOffset":null}));
    for (name, cache) in caches {
        if name.is_empty()
            || name.len() > 100
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
            || !cache.is_object()
        {
            return Err(Error::new("Invalid stored environment cache."));
        }
        let ids = match cache.get("itemIds") {
            None => &[][..],
            Some(Value::Array(ids)) if ids.len() <= 2000 => ids.as_slice(),
            _ => return Err(Error::new("Invalid stored environment membership.")),
        };
        if ids.iter().any(|id| {
            !id.as_str().is_some_and(|id| {
                id.starts_with("sentry:") && id.len() <= 200 && !id.chars().any(char::is_control)
            })
        }) {
            return Err(Error::new("Invalid stored environment member ID."));
        }
        summaries.push(json!({"environment":name,"cached":true,"refreshing":cache["running"]==true,
            "lastSuccess":valid_time(cache["lastSuccess"].as_f64()),"nextAttempt":valid_time(cache["nextAttempt"].as_f64()),
            "failures":failures(cache),"hasError":cache["error"].is_string(),"membershipKnown":cache.get("itemIds").is_some(),"cachedMemberCount":cache.get("itemIds").map(|_| ids.len())}));
        if requested == Some(name.as_str()) {
            selected = Some(
                json!({"environment":name,"cached":true,"membershipKnown":cache.get("itemIds").is_some(),"itemIds":ids.iter().skip(offset).take(100).collect::<Vec<_>>(),
                "total":cache.get("itemIds").map(|_| ids.len()),"offset":offset,"nextOffset":(offset+100 < ids.len()).then_some(offset+100)}),
            );
        }
    }
    let source = &state["sources"]["sentry"];
    Ok(
        json!({"version":1,"scope":"cached","observedAt":observed_at,"demo":demo,
        "source":{"syncEnabled":source["disabled"]!=true,"hasError":source["error"].is_string(),"failures":failures(source),"lastSuccess":valid_time(source["lastSuccess"].as_f64()),"nextAttempt":valid_time(source["nextAttempt"].as_f64())},
        "environmentRequestsAfter":valid_time(state["sentryEnvNextAttempt"].as_f64()),"environments":summaries,"selection":selected}),
    )
}

#[cfg(test)]
mod tests;
