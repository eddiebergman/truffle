//! Sole writer of the canonical JSON state. UI projections never write layout into records.
use super::{arr, json, now, text, uuid, Error, PathBuf, Result, Value};
use super::{connections::ProviderHost, domain, sessions, source_sync::SourceSync};
use fs2::FileExt;
use std::{
    collections::BTreeSet,
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

pub struct Repository {
    pub settings: super::settings::Store,
    pub app_chat: Arc<super::app_chat::Host>,
    pub quick_actions: std::sync::Arc<super::quick_actions::Host>,
    pub root: PathBuf,
    pub agent_helper: String,
    pub data: PathBuf,
    pub state: Mutex<Value>,
    pub providers: Arc<ProviderHost>,
    pub sync: SourceSync,
    pub terminals: super::terminals::Terminals,
    pub shells: super::terminals::Terminals,
    pub demo: bool,
    pub git_diff_cache: Mutex<super::git_diff::Cache>,
    pub gcp_auth: Arc<super::gcp_auth::LoginHost>,
    pub cooldown: Mutex<std::collections::BTreeMap<String, f64>>,
    pub opening: Mutex<BTreeSet<String>>,
    pub sharing: Mutex<Value>,
    pub sharing_busy: Mutex<BTreeSet<String>>,
    folders_cache: Mutex<Option<(f64, Value)>>,
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
        let providers = Arc::new(ProviderHost::new(&root, data.parent().unwrap())?);
        state["sources"]["sentry"]["disabled"] = json!(!providers.sentry_configured());
        let sharing = super::sharing::load(data.parent().unwrap())?;
        let terminals = super::terminals::Terminals::new(super::terminal_daemon::Host::new(
            &root, &data, demo,
        )?);
        let settings = super::settings::Store::load(data.parent().unwrap().join("settings.json"))?;
        let repo = Arc::new(Self {
            settings,
            app_chat: Arc::new(super::app_chat::Host::default()),
            quick_actions: std::sync::Arc::new(super::quick_actions::Host::load(
                data.parent().unwrap().join("quick-actions.json"),
            )?),
            shells: super::terminals::Terminals::new(super::terminal_daemon::Host::new(
                &root,
                &data.parent().unwrap().join("shell-terminals.json"),
                demo,
            )?),
            root,
            agent_helper,
            data,
            state: Mutex::new(state),
            sync: SourceSync::new(providers.clone()),
            providers,
            terminals,
            demo,
            git_diff_cache: Mutex::new(super::git_diff::Cache::default()),
            gcp_auth: Arc::new(super::gcp_auth::LoginHost::default()),
            cooldown: Mutex::new(std::collections::BTreeMap::new()),
            opening: Mutex::new(BTreeSet::new()),
            sharing: Mutex::new(sharing),
            sharing_busy: Mutex::new(BTreeSet::new()),
            folders_cache: Mutex::new(None),
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
    /// Publish a change that lives outside the canonical state, such as sync progress.
    pub(super) fn bump_revision(&self) {
        self.revision.fetch_add(1, Ordering::Relaxed);
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
        v["syncing"] = json!(self.sync.syncing());
        v["forceRefreshAfter"] = json!(self.sync.force_after());
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
    /// The live terminal list for the UI. A daemon failure becomes a warning.
    pub fn inventory(&self) -> Value {
        let (candidates, warning) = match self.terminals.inventory() {
            Ok(inventory) => (inventory.rows().to_vec(), None),
            Err(error) => (vec![], Some(error.message)),
        };
        let folders = {
            let mut cache = self.folders_cache.lock().unwrap();
            match &*cache {
                Some((until, folders)) if *until > now() => folders.clone(),
                _ => {
                    let folders = super::workflows::folders();
                    *cache = Some((now() + 4.0, folders.clone()));
                    folders
                }
            }
        };
        json!({"candidates":candidates,"directories":arr(&folders).iter().map(|x|x["path"].clone()).collect::<Vec<_>>(),"folders":folders,"workRoot":super::workflows::work_root(),"warning":warning,"demo":self.demo})
    }
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
