//! Sole writer of the canonical JSON state. UI projections never write layout into records.
use super::{arr, json, now, text, uuid, Error, PathBuf, Result, Value};
use super::{connections::ProviderHost, domain, sessions};
use fs2::FileExt;
use std::{
    collections::{BTreeSet, HashSet},
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    sync::atomic::{AtomicU64, Ordering},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
// Closing a file descriptor alone does not release flock while a forked child
// still holds a duplicate. Unlock explicitly when the owning repository ends.
struct DataFileLock(File);
impl DataFileLock {
    fn acquire(file: File) -> Result<Self> {
        file.try_lock_exclusive()
            .map_err(|_| Error::new("Another truffle service is using this data file."))?;
        Ok(Self(file))
    }
}
impl Drop for DataFileLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

fn source_time(value: &Value) -> Value {
    value
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0.0 && *value <= 9_007_199_254_740_991.0)
        .map_or(Value::Null, |value| json!(value))
}
fn source_failures(info: &Value) -> u64 {
    info["failures"]
        .as_u64()
        .filter(|value| *value <= 9_007_199_254_740_991)
        .unwrap_or(0)
}
pub(super) fn source_skip(
    info: &Value,
    running: bool,
    force: bool,
    time: f64,
) -> Option<(&'static str, Option<f64>)> {
    if info["disabled"] == true {
        return Some(("disabled", None));
    }
    if running {
        return Some(("already-running", None));
    }
    if let Some(next) = source_time(&info["nextAttempt"])
        .as_f64()
        .filter(|next| *next > time)
    {
        if info["failures"].as_u64().unwrap_or(0) > 0 {
            return Some(("backoff", Some(next)));
        }
        if !force {
            return Some(("interval", Some(next)));
        }
    }
    None
}

pub struct Repository {
    pub settings: super::settings::Store,
    pub app_chat: Arc<super::app_chat::Host>,
    pub quick_actions: std::sync::Arc<super::quick_actions::Host>,
    pub root: PathBuf,
    pub agent_helper: String,
    pub data: PathBuf,
    pub state: Mutex<Value>,
    pub providers: ProviderHost,
    pub embedded: Arc<super::terminal_daemon::Host>,
    pub shells: Arc<super::terminal_daemon::Host>,
    pub demo: bool,
    pub git_diff_cache: Mutex<super::git_diff::Cache>,
    pub gcp_auth: Arc<super::gcp_auth::LoginHost>,
    pub running: Mutex<BTreeSet<String>>,
    pub cooldown: Mutex<std::collections::BTreeMap<String, f64>>,
    pub opening: Mutex<BTreeSet<String>>,
    pub sharing: Mutex<Value>,
    pub sharing_busy: Mutex<BTreeSet<String>>,
    inventory_cache: Mutex<Option<(f64, Value)>>,
    revision: AtomicU64,
    generation: String,
    snapshot_cache: Mutex<Option<(u64, f64, Value)>>,
    _lock: DataFileLock,
}
impl Repository {
    pub fn open(root: PathBuf, path: Option<PathBuf>, demo: bool) -> Result<Arc<Self>> {
        let data = path.unwrap_or_else(|| root.join("data/state.json"));
        std::fs::create_dir_all(data.parent().unwrap())?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(data.with_extension("lock"))?;
        let lock = DataFileLock::acquire(lock)?;
        let mut state = if data.is_file() {
            serde_json::from_slice(&std::fs::read(&data)?)?
        } else {
            json!({"version":1})
        };
        domain::initialize(&mut state)?;
        let agent_helper = super::agent_cli::helper(&root)?;
        let docs = data.parent().unwrap().join("runtime-docs");
        std::fs::create_dir_all(&docs)?;
        super::http::private_write(
            &docs.join("truffle.md"),
            include_bytes!("../../../skill/SKILL.md"),
        )?;
        let providers = ProviderHost::new(&root, data.parent().unwrap())?;
        state["sources"]["sentry"]["disabled"] = json!(!providers.sentry_configured());
        let sharing = super::sharing::load(data.parent().unwrap())?;
        let embedded = Arc::new(super::terminal_daemon::Host::new(&root, &data, demo)?);
        let settings = super::settings::Store::load(data.parent().unwrap().join("settings.json"))?;
        let repo = Arc::new(Self {
            settings,
            app_chat: Arc::new(super::app_chat::Host::default()),
            quick_actions: std::sync::Arc::new(super::quick_actions::Host::load(
                data.parent().unwrap().join("quick-actions.json"),
            )?),
            shells: Arc::new(super::terminal_daemon::Host::new(
                &root,
                &data.parent().unwrap().join("shell-terminals.json"),
                demo,
            )?),
            root,
            agent_helper,
            data,
            state: Mutex::new(state),
            providers,
            embedded,
            demo,
            git_diff_cache: Mutex::new(super::git_diff::Cache::default()),
            gcp_auth: Arc::new(super::gcp_auth::LoginHost::default()),
            running: Mutex::new(BTreeSet::new()),
            cooldown: Mutex::new(std::collections::BTreeMap::new()),
            opening: Mutex::new(BTreeSet::new()),
            sharing: Mutex::new(sharing),
            sharing_busy: Mutex::new(BTreeSet::new()),
            inventory_cache: Mutex::new(None),
            revision: AtomicU64::new(1),
            generation: uuid(),
            snapshot_cache: Mutex::new(None),
            _lock: lock,
        });
        super::resources::import_pending(&repo);
        Ok(repo)
    }
    pub fn save(&self, state: &Value) -> Result<()> {
        let temp = self.data.with_extension("tmp");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(&serde_json::to_vec_pretty(state)?)?;
        file.sync_all()?;
        std::fs::rename(temp, &self.data)?;
        Ok(())
    }
    /// One-time recovery artifact only. Call under the canonical transaction lock.
    /// Never loaded as live state or included in public state/export projections.
    pub(super) fn backup_dispatch_v1(&self, journal: &Value) -> Result<()> {
        const LIMIT: usize = 4 * 1024 * 1024;
        let bytes = serde_json::to_vec(journal)?;
        if journal["version"] != 1 || bytes.len() > LIMIT {
            return Err(Error::new(
                "Invalid legacy dispatch recovery data. Migration was not applied.",
            ));
        }
        let path = self
            .data
            .with_extension("session-dispatch-v1-recovery.json");
        let parent = path.parent().ok_or_else(Error::io)?;
        let parent_metadata = std::fs::metadata(parent)?;
        if !parent_metadata.is_dir() || parent_metadata.mode() & 0o022 != 0 {
            return Err(Error::new(
                "Dispatch recovery directory is not private enough. Migration was not applied.",
            ));
        }
        let owner = parent_metadata.uid();
        let verify = || -> Result<bool> {
            let mut file = match OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(&path)
            {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(_) => {
                    return Err(Error::new(
                        "Dispatch recovery backup is unavailable. Migration was not applied.",
                    ))
                }
            };
            let metadata = file.metadata()?;
            if !metadata.is_file()
                || metadata.uid() != owner
                || metadata.permissions().mode() & 0o777 != 0o600
                || metadata.len() > LIMIT as u64
            {
                return Err(Error::new("Dispatch recovery backup is not a private regular file. Migration was not applied."));
            }
            let mut stored = Vec::new();
            Read::by_ref(&mut file)
                .take(LIMIT as u64 + 1)
                .read_to_end(&mut stored)?;
            if stored != bytes {
                return Err(Error::new("An existing dispatch recovery backup conflicts with this migration. Both copies were kept."));
            }
            file.sync_all()?;
            Ok(true)
        };
        if verify()? {
            File::open(parent)?.sync_all()?;
            return Ok(());
        }
        // A crashed prior attempt must not block migration; only the fixed final artifact is authoritative recovery.
        let temp = path.with_extension(format!("tmp-{}", uuid()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temp)
            .map_err(|_| {
                Error::new(
                    "Dispatch recovery backup could not be created. Migration was not applied.",
                )
            })?;
        let result = (|| -> Result<()> {
            file.write_all(&bytes)?;
            file.sync_all()?;
            // Atomic no-clobber publication; another existing recovery artifact is never replaced.
            match std::fs::hard_link(&temp, &path) {
                Ok(()) => (),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if !verify()? {
                        return Err(Error::io());
                    }
                }
                Err(error) => return Err(error.into()),
            }
            Ok(())
        })();
        drop(file);
        let cleanup = std::fs::remove_file(&temp);
        result?;
        cleanup?;
        File::open(parent)?.sync_all()?;
        Ok(())
    }
    pub fn transaction<T>(&self, f: impl FnOnce(&mut Value) -> Result<T>) -> Result<T> {
        self.commit_transaction(f, false)
    }
    /// Guard and commit one conditional refresh atomically, without writing an unchanged cache.
    pub(super) fn transaction_if_changed<T>(
        &self,
        f: impl FnOnce(&mut Value) -> Result<T>,
    ) -> Result<T> {
        self.commit_transaction(f, true)
    }
    fn commit_transaction<T>(
        &self,
        f: impl FnOnce(&mut Value) -> Result<T>,
        skip_unchanged: bool,
    ) -> Result<T> {
        let mut state = self.state.lock().map_err(|_| Error::io())?;
        self.commit_locked(&mut state, f, skip_unchanged)
    }
    /// Commit `f` against `state`, which the caller has locked from `self.state`
    /// and holds through commit and publication.
    pub(super) fn commit_locked<T>(
        &self,
        state: &mut Value,
        f: impl FnOnce(&mut Value) -> Result<T>,
        skip_unchanged: bool,
    ) -> Result<T> {
        let mut next = state.clone();
        let result = f(&mut next)?;
        if !skip_unchanged || next != *state {
            self.save(&next)?;
            *state = next;
            self.revision.fetch_add(1, Ordering::Relaxed);
        }
        Ok(result)
    }
    pub fn invalidate_snapshot(&self) {
        *self.snapshot_cache.lock().unwrap() = None;
    }
    pub fn snapshot(&self, export: bool) -> Value {
        if !export {
            let revision = self.revision.load(Ordering::Relaxed);
            if let Some((cached, expires, value)) = &*self.snapshot_cache.lock().unwrap() {
                if *cached == revision && *expires > now() {
                    return value.clone();
                }
            }
        }
        let s = self.state.lock().unwrap();
        let revision = self.revision.load(Ordering::Relaxed);
        let (v, expires) = self.project_snapshot(&s, export, revision);
        drop(s);
        if !export {
            *self.snapshot_cache.lock().unwrap() = Some((revision, expires, v.clone()));
        }
        v
    }
    // Capture raw owner input and its existing public projection at one boundary.
    pub(super) fn capture_for_open(&self) -> Result<(Value, Value)> {
        let state = self.state.lock().map_err(|_| Error::io())?;
        let public = self
            .project_snapshot(&state, false, self.revision.load(Ordering::Relaxed))
            .0;
        Ok((state.clone(), public))
    }
    fn project_snapshot(&self, s: &Value, export: bool, revision: u64) -> (Value, f64) {
        let time = now();
        let mut expires = f64::INFINITY;
        let mut expired_snoozes = 0;
        for snooze in s["snoozes"].as_object().unwrap().values() {
            if let Some(until) = snooze["until"].as_f64().filter(|n| *n > 0.0) {
                if until > time {
                    expires = expires.min(until);
                } else {
                    expired_snoozes += 1;
                }
            }
        }
        let mut v = s.clone();
        v.as_object_mut().unwrap().remove("sessionDispatch");
        v.as_object_mut().unwrap().remove("uiPresentations");
        let catalog = domain::catalog(s);
        for session in sessions::all_mut(&mut v) {
            if !export {
                sessions::public(session);
            }
            session["items"] = json!(arr(&session["items"])
                .iter()
                .map(|i| domain::item_ref(catalog.get(text(i, "id")).unwrap_or(i)))
                .collect::<Vec<_>>());
        }
        v["bundles"] = domain::bundles(s);
        let ids: BTreeSet<String> = arr(&v["bundles"])
            .iter()
            .flat_map(|x| arr(&x["itemIds"]))
            .filter_map(|x| x.as_str().map(str::to_owned))
            .collect();
        let current: BTreeSet<String> = arr(&s["items"])
            .iter()
            .chain(arr(&s["notes"]).iter())
            .map(|x| text(x, "id").to_owned())
            .collect();
        v["relatedItems"] = json!(catalog
            .iter()
            .filter(|(id, _)| ids.contains(*id) && !current.contains(*id))
            .map(|(_, item)| domain::item_ref(item))
            .collect::<Vec<_>>());
        for collection in ["items", "notes"] {
            for item in v[collection].as_array_mut().unwrap() {
                item["snoozed"] = json!(domain::snoozed(s, item));
                item["actionReasons"] = json!(domain::reasons(item));
            }
        }
        if !export {
            v.as_object_mut().unwrap().remove("itemArchive");
            v.as_object_mut().unwrap().remove("changeBaselines");
            v.as_object_mut().unwrap().remove("onboardingRecovery");
        }
        v["syncing"] = json!(self.running.lock().unwrap().iter().collect::<Vec<_>>());
        v["forceRefreshAfter"] = json!(self
            .cooldown
            .lock()
            .unwrap()
            .get("force-refresh")
            .unwrap_or(&0.0));
        v["revision"] = json!(format!("{}:{revision}:{expired_snoozes}", self.generation));
        v["demo"] = json!(self.demo);
        v["agentHelper"] = json!(self.agent_helper);
        v["interval"] = json!(300);
        if self.providers.sentry_configured() {
            v["sentryScope"] = self.providers.metadata();
        }
        (v, expires)
    }
    pub fn cooldown(&self, key: &str, seconds: f64) -> Result<()> {
        let mut times = self.cooldown.lock().unwrap();
        if times.get(key).copied().unwrap_or(0.0) > now() {
            return Err(Error::new("This action just started. Give it a moment."));
        }
        times.insert(key.into(), now() + seconds);
        drop(times);
        Ok(())
    }
    pub fn edit(&self, p: &Value) -> Result<()> {
        if text(p, "action") == "save-session" && !text(p, "id").is_empty() {
            super::chat_edit::legacy(self, p)?;
            return Ok(());
        }
        self.transaction(|s| match text(p, "action") {
            "save-session" => {
                super::chat_edit::reject_unchecked_association(s, p)?;
                super::sessions::save(s, p)?;
                Ok(())
            }
            "save-artifact" => {
                if p.get("id").is_some() {
                    super::artifacts::legacy_metadata_update(self, s, p)?;
                } else {
                    super::artifacts::save(self, s, p)?;
                }
                Ok(())
            }
            _ => domain::edit(s, p),
        })
    }
    /// Bounded sync metadata. Never expose provider configuration or arbitrary source fields.
    pub fn sources_status(&self) -> Value {
        let mut sources = {
            let state = self.state.lock().unwrap();
            ["github", "linear", "sentry"].into_iter().map(|id| {
                let info = &state["sources"][id];
                json!({"id":id,"syncEnabled":info["disabled"]!=true,
                    "lastSuccess":source_time(&info["lastSuccess"]),"nextAttempt":source_time(&info["nextAttempt"]),
                    "failures":source_failures(info),"hasError":info["error"].is_string(),
                    "error":info["error"].as_str(),
                    "cachedItemCount":state["items"].as_array().map_or(0,|items|items.iter().filter(|item|item["source"]==id).count())})
            }).collect::<Vec<_>>()
        };
        let running = self.running.lock().unwrap().clone();
        let force_after = self
            .cooldown
            .lock()
            .unwrap()
            .get("force-refresh")
            .copied()
            .unwrap_or(0.0);
        for row in &mut sources {
            row["refreshing"] = json!(running.contains(text(row, "id")));
            if let Some(error) = row["error"].as_str() {
                let mut error = error.to_owned();
                super::commands::redact_text(self, &mut error);
                row["error"] = json!(error.chars().take(2000).collect::<String>());
            }
        }
        json!({"observedAt":now(),"demo":self.demo,"forceRefreshAfter":source_time(&json!(force_after)),"sources":sources})
    }
    pub fn sentry_environments_read(&self, args: &Value) -> Result<Value> {
        let requested = args["environment"].as_str();
        let offset = usize::try_from(args["offset"].as_u64().unwrap_or(0)).unwrap_or(usize::MAX);
        if requested.is_none() && args.get("offset").is_some() {
            return Err(Error::new(
                "An environment is required for membership pagination.",
            ));
        }
        let observed_at = now();
        let mut result = sentry_environment_summary(
            &*self.state.lock().map_err(|_| Error::io())?,
            requested,
            offset,
            observed_at,
            self.demo,
        )?;
        let original = result.clone();
        super::commands::redact(self, &mut result);
        if result != original {
            return Err(Error::new(
                "Environment identifiers cannot be returned safely.",
            ));
        }
        Ok(result)
    }
    pub fn schedule(self: &Arc<Self>, force: bool) -> Result<Vec<String>> {
        let receipt = self.refresh_sources(force)?;
        Ok(arr(&receipt["results"])
            .into_iter()
            .filter(|row| row["status"] == "started")
            .map(|row| text(&row, "id").to_owned())
            .collect())
    }
    pub fn refresh_sources(self: &Arc<Self>, force: bool) -> Result<Value> {
        self.refresh_sources_with(force, |repo, source| {
            std::thread::spawn(move || repo.sync(source));
        })
    }
    fn refresh_sources_with(
        self: &Arc<Self>,
        force: bool,
        launch: impl Fn(Arc<Self>, &'static str),
    ) -> Result<Value> {
        if !self.demo && force {
            self.cooldown("force-refresh", 15.0)?;
            // A no-op force request still changes its cooldown and must invalidate cached snapshots.
            self.revision.fetch_add(1, Ordering::Relaxed);
        }
        let time = now();
        let mut results = vec![];
        let mut started = vec![];
        if self.demo {
            for id in ["github", "linear", "sentry"] {
                results.push(json!({"id":id,"status":"skipped","reason":"demo"}));
            }
        } else {
            let mut state = self.state.lock().unwrap();
            let mut running = self.running.lock().unwrap();
            for source in ["github", "linear", "sentry"] {
                let info = &mut state["sources"][source];
                if let Some((reason, retry_at)) =
                    source_skip(info, running.contains(source), force, time)
                {
                    let mut row = json!({"id":source,"status":"skipped","reason":reason});
                    if let Some(at) = retry_at {
                        row["retryAt"] = json!(at);
                    }
                    results.push(row);
                    continue;
                }
                running.insert(source.into());
                info["nextAttempt"] = json!(time + 300.0);
                started.push(source);
                results.push(json!({"id":source,"status":"started"}));
            }
            if !started.is_empty() {
                self.revision.fetch_add(1, Ordering::Relaxed);
            }
            drop(running);
            drop(state);
        }
        // Reservations are complete before workers start; never hold state locks across provider I/O.
        for source in started.iter().copied() {
            launch(self.clone(), source);
        }
        let message = if self.demo {
            "Sample workspace: no external requests were started.".to_owned()
        } else if started.is_empty() {
            "Nothing started: sources are disabled, already refreshing, or waiting for their retry window.".to_owned()
        } else {
            format!("Refreshing {}…", started.join(" & "))
        };
        let force_after = self
            .cooldown
            .lock()
            .unwrap()
            .get("force-refresh")
            .copied()
            .unwrap_or(0.0);
        Ok(
            json!({"status":if started.is_empty(){"skipped"}else{"started"},"force":force,
            "forceRefreshAfter":source_time(&json!(force_after)),"results":results,"message":message}),
        )
    }
    fn sync(self: Arc<Self>, source: &str) {
        let start = now();
        let result: Result<usize> = (|| {
            let mut items = self.providers.fetch(source, None)?;
            let refs = {
                let state = self.state.lock().unwrap();
                let catalog = domain::catalog(&state);
                let mut ids = BTreeSet::new();
                for session in sessions::all(&state) {
                    if session["archivedAt"].is_null() {
                        for i in arr(&session["items"]) {
                            if i["source"] == source {
                                ids.insert(text(&i, "id").to_string());
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
                            || ["github", "linear"].contains(&source)
                                && item["status"] != "done"
                                && current.contains(id.as_str()))
                    {
                        ids.insert(id.clone());
                    }
                }
                let fetched = items.iter().map(|x| text(x, "id")).collect::<HashSet<_>>();
                ids.into_iter()
                    .filter(|id| !fetched.contains(id.as_str()))
                    .filter_map(|id| catalog.get(&id).cloned())
                    .collect::<Vec<_>>()
            };
            if !refs.is_empty() {
                items.extend(self.providers.refresh(source, &refs)?);
            }
            self.transaction(|s|{let mut by_id=items.iter().map(|x|(text(x,"id").to_string(),x.clone())).collect::<std::collections::BTreeMap<_,_>>();for item in arr(&s["items"]){if item["source"]==source&&s["overrides"][text(&item,"id")]["hidden"]==true{by_id.entry(text(&item,"id").into()).or_insert(item);}}items=by_id.into_values().collect();for session in sessions::all_mut(s){for item in session["items"].as_array_mut().unwrap(){if let Some(new)=items.iter().find(|x|x["id"]==item["id"]){*item=domain::item_ref(new);}}}for item in &items{if refs.iter().any(|x|x["id"]==item["id"]){s["itemArchive"][text(item,"id")]=item.clone();}}let mut all=arr(&s["items"]);all.retain(|x|x["source"]!=source);all.extend(items.clone());s["items"]=json!(all);domain::observe(s,source,&items);if source=="github"{super::github::observe(s);}s["sources"][source]=json!({"lastSuccess":now(),"nextAttempt":now()+300.0,"error":null,"failures":0,"disabled":false});Ok(())})?;
            Ok(items.len())
        })();
        let mut event = json!({"time":iso_time(),"source":source,"operation":"sync","elapsedMs":((now()-start)*1000.0).round()});
        match result {
            Ok(count) => {
                event["outcome"] = json!("success");
                event["itemCount"] = json!(count);
            }
            Err(error) => {
                let mut retry = 300.0;
                let _ = self.transaction(|s| {
                    let info = &mut s["sources"][source];
                    let n = info["failures"].as_u64().unwrap_or(0) + 1;
                    retry = (300.0 * 2.0f64.powi((n - 1).min(4) as i32))
                        .min(3600.0)
                        .max(error.retry);
                    info["failures"] = json!(n);
                    info["error"] = json!(error.message);
                    info["nextAttempt"] = json!(now() + retry);
                    Ok(())
                });
                event["outcome"] = json!("failure");
                event["retrySeconds"] = json!(retry);
                if let Some(fields) = error.data.as_object() {
                    for (k, v) in fields {
                        event[k] = v.clone();
                    }
                }
            }
        }
        let _ = self.log(&event);
        self.running.lock().unwrap().remove(source);
        self.revision.fetch_add(1, Ordering::Relaxed);
    }
    pub fn log(&self, event: &Value) -> Result<()> {
        let root = self.data.parent().unwrap().join("logs");
        std::fs::create_dir_all(&root)?;
        let path = root.join("sync.jsonl");
        if path.metadata().is_ok_and(|m| m.len() > 512 * 1024) {
            let _ = std::fs::rename(root.join("sync.jsonl.1"), root.join("sync.jsonl.2"));
            let _ = std::fs::rename(&path, root.join("sync.jsonl.1"));
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)?;
        writeln!(file, "{event}")?;
        Ok(())
    }
    pub fn logs(&self) -> Value {
        let root = self.data.parent().unwrap().join("logs");
        let mut events = vec![];
        for name in ["sync.jsonl", "sync.jsonl.1", "sync.jsonl.2"] {
            if let Ok(data) = std::fs::read_to_string(root.join(name)) {
                for line in data.lines().rev() {
                    if let Ok(value) = serde_json::from_str::<Value>(line) {
                        events.push(value);
                    }
                    if events.len() >= 500 {
                        break;
                    }
                }
            }
            if events.len() >= 500 {
                break;
            }
        }
        json!({"entries":events,"limit":500,"path":"data/logs/sync.jsonl"})
    }
    pub fn inventory(&self) -> Value {
        let mut cache = self.inventory_cache.lock().unwrap();
        if let Some((until, value)) = &*cache {
            if *until > now() {
                let mut value = value.clone();
                let mut candidates = arr(&value["candidates"]);
                candidates.retain(|x| x["host"] != "embedded");
                candidates.extend(self.embedded.inventory());
                value["candidates"] = json!(candidates);
                value["warning"] = json!(self.embedded.warning());
                return value;
            }
        }
        let folders = super::workflows::folders();
        let value = json!({"candidates":self.embedded.inventory(),"folders":folders,"directories":arr(&folders).iter().map(|x|x["path"].clone()).collect::<Vec<_>>(),"workRoot":super::workflows::work_root(),"warning":self.embedded.warning(),"demo":self.demo});
        *cache = Some((now() + 4.0, value.clone()));
        value
    }
}
fn sentry_environment_summary(
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
            "lastSuccess":source_time(&cache["lastSuccess"]),"nextAttempt":source_time(&cache["nextAttempt"]),
            "failures":source_failures(cache),"hasError":cache["error"].is_string(),"membershipKnown":cache.get("itemIds").is_some(),"cachedMemberCount":cache.get("itemIds").map(|_| ids.len())}));
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
        "source":{"syncEnabled":source["disabled"]!=true,"hasError":source["error"].is_string(),"failures":source_failures(source),"lastSuccess":source_time(&source["lastSuccess"]),"nextAttempt":source_time(&source["nextAttempt"])},
        "environmentRequestsAfter":source_time(&state["sentryEnvNextAttempt"]),"environments":summaries,"selection":selected}),
    )
}
pub fn iso_time() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        });
    let mut t: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::gmtime_r(&raw const secs, &raw mut t);
    }
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}+00:00",
        t.tm_year + 1900,
        t.tm_mon + 1,
        t.tm_mday,
        t.tm_hour,
        t.tm_min,
        t.tm_sec
    )
}

#[cfg(test)]
mod tests;
