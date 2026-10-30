//! Native credential setup. Public projections contain no credentials.
use super::source_sync::ProviderSource;
use super::{json, now, text, uuid, Error, PathBuf, Repository, Result, Value};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
    time::Duration,
};
use zeroize::Zeroizing;

const SERVICE: &str = "app.truffle.connections.v1";
const STORAGE: &str = "macos-login-keychain";
const MAX_CONFIG: u64 = 16_384;
const MAX_SECRET: usize = 8192;
const TTL: f64 = 300.0;
const PROVIDERS: [&str; 3] = ["github", "linear", "sentry"];
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    credential_ref: String,
    identity: String,
    #[serde(default)]
    org: String,
    #[serde(default)]
    base: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    version: u32,
    storage: String,
    revision: u64,
    providers: BTreeMap<String, Entry>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            storage: STORAGE.into(),
            revision: 0,
            providers: BTreeMap::new(),
        }
    }
}
fn config_path(profile: &Path) -> PathBuf {
    profile.join("config/connections.json")
}
fn load_config(profile: &Path) -> Result<Config> {
    let path = config_path(profile);
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(_) => return Err(Error::new("Connection configuration cannot be read.")),
    };
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CONFIG {
        return Err(Error::new(
            "Connection configuration exceeds its size limit.",
        ));
    }
    let config: Config = serde_json::from_slice(&bytes).map_err(|_| {
        Error::new(
            "Connection configuration is invalid or uses unsupported fields. It has not changed.",
        )
    })?;
    if config.version != 1 || config.revision == u64::MAX {
        return Err(Error::new(
            "Unsupported connection configuration version or revision. It has not changed.",
        ));
    }
    if config.storage != STORAGE {
        return Err(Error::new(
            "Unsupported credential storage backend. Connections have not changed.",
        ));
    }
    for (provider, entry) in &config.providers {
        if !PROVIDERS.contains(&provider.as_str())
            || uuid::Uuid::parse_str(&entry.credential_ref).is_err()
            || entry.identity.is_empty()
            || entry.identity.len() > 200
        {
            return Err(Error::new("Invalid connection reference."));
        }
        validate_scope(provider, &entry.org, &entry.base)?;
    }
    Ok(config)
}
fn config_view(config: &Config) -> Value {
    json!({"available":cfg!(target_os="macos"),"storage":STORAGE,"revision":config.revision,"providers":PROVIDERS.iter().map(|provider| {
        let entry = config.providers.get(*provider);
        json!({"id":provider,"configured":entry.is_some(),"identity":entry.map(|entry| &entry.identity),"org":entry.map(|entry| &entry.org),"base":entry.map(|entry| &entry.base)})
    }).collect::<Vec<_>>()})
}
fn validate_scope(provider: &str, org: &str, base: &str) -> Result<()> {
    if !PROVIDERS.contains(&provider) {
        return Err(Error::new("Unknown connection provider."));
    }
    match provider {
        "sentry"
            if org.is_empty()
                || org.len() > 100
                || !org
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_-".contains(c))
                || ![
                    "https://sentry.io",
                    "https://us.sentry.io",
                    "https://de.sentry.io",
                ]
                .contains(&base) =>
        {
            Err(Error::new(
                "Choose a Sentry cloud region and enter an organization slug.",
            ))
        }
        "github" | "linear" if !org.is_empty() || !base.is_empty() => Err(Error::new(
            "This provider does not accept a custom host or organization.",
        )),
        _ => Ok(()),
    }
}
fn secret(value: &str) -> Result<Zeroizing<String>> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAX_SECRET
        || !value.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(Error::new(
            "Enter a token without spaces or line breaks (maximum 8 KiB).",
        ));
    }
    Ok(Zeroizing::new(value.to_owned()))
}
trait CredentialStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Zeroizing<String>>;
    fn put(&self, account: &str, token: &str) -> Result<()>;
    fn delete(&self, account: &str) -> Result<()>;
}
struct Keychain;
#[cfg(target_os = "macos")]
fn password_options(account: &str) -> security_framework::passwords::PasswordOptions {
    let mut options =
        security_framework::passwords::PasswordOptions::new_generic_password(SERVICE, account);
    options.set_access_synchronized(Some(false));
    options
}
impl CredentialStore for Keychain {
    fn get(&self, account: &str) -> Result<Zeroizing<String>> {
        #[cfg(target_os = "macos")]
        {
            let bytes = Zeroizing::new(security_framework::passwords::generic_password(password_options(account)).map_err(|_| Error::new("Cannot read this connection from macOS Keychain. Unlock Keychain or reconnect. No other account was used."))?);
            secret(std::str::from_utf8(&bytes).map_err(|_| {
                Error::new("The Keychain connection is invalid. Reconnect this account.")
            })?)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = account;
            Err(Error::new(
                "Secure connection storage requires macOS Keychain.",
            ))
        }
    }
    fn put(&self, account: &str, token: &str) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            security_framework::passwords::set_generic_password_options(token.as_bytes(), password_options(account)).map_err(|_| Error::new("macOS Keychain could not save this connection. Nothing was written to a plaintext file."))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (account, token);
            Err(Error::new(
                "Secure connection storage requires macOS Keychain.",
            ))
        }
    }
    fn delete(&self, account: &str) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            security_framework::passwords::delete_generic_password_options(password_options(
                account,
            ))
            .map_err(|_| Error::new("The unused Keychain connection could not be removed."))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = account;
            Err(Error::new(
                "Secure connection storage requires macOS Keychain.",
            ))
        }
    }
}
fn account(profile: &Path, reference: &str) -> String {
    format!(
        "{:x}:{reference}",
        Sha256::digest(profile.as_os_str().as_encoded_bytes())
    )
}
trait Remote: Send + Sync {
    fn preview(&self, provider: &str, token: &str, org: &str, base: &str) -> Result<Value>;
}
struct ProviderRemote {
    client: ureq::Agent,
}
impl ProviderRemote {
    fn new() -> Self {
        Self {
            client: super::http_config()
                .user_agent("truffle-connections/1")
                .proxy(None)
                .timeout_global(Some(Duration::from_secs(20)))
                .build()
                .into(),
        }
    }
    fn json(&self, url: &str, token: &str, query: Option<&str>) -> Result<Value> {
        let mut auth = ureq::http::HeaderValue::from_str(token)
            .map_err(|_| Error::new("Invalid token format."))?;
        auth.set_sensitive(true);
        let response = match query {
            None => self
                .client
                .get(url)
                .header(ureq::http::header::AUTHORIZATION, auth)
                .call(),
            Some(query) => self
                .client
                .post(url)
                .header(ureq::http::header::AUTHORIZATION, auth)
                .header("Content-Type", "application/json")
                .send(&serde_json::to_vec(&json!({"query":query}))?),
        }
        .map_err(|_| {
            Error::new("Connection validation failed. Check your network and try again.")
        })?;
        if !response.status().is_success() {
            return Err(Error::new(format!("The provider returned HTTP {}. Check token permissions, expiration and organization approval.", response.status().as_u16())));
        }
        let mut bytes = Zeroizing::new(Vec::new());
        response
            .into_body()
            .into_reader()
            .take(524_289)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 524_288 {
            return Err(Error::new("The provider preview exceeded its size limit."));
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| Error::new("The provider returned an invalid preview."))?;
        if value.get("errors").is_some() {
            return Err(Error::new(
                "The provider could not verify the requested access. Check token permissions.",
            ));
        }
        Ok(value)
    }
}
fn public_text(value: &Value, key: &str) -> Result<String> {
    let value = value[key]
        .as_str()
        .ok_or_else(|| Error::new("The provider returned an incomplete identity or resource."))?;
    if value.is_empty() || value.len() > 200 || value.chars().any(char::is_control) {
        return Err(Error::new(
            "The provider returned an invalid identity or resource.",
        ));
    }
    Ok(value.into())
}
fn names(value: &Value, key: &str) -> Result<Vec<String>> {
    let rows = value
        .as_array()
        .ok_or_else(|| Error::new("The provider returned an invalid resource list."))?;
    rows.iter()
        .take(20)
        .map(|row| public_text(row, key))
        .collect()
}
impl Remote for ProviderRemote {
    fn preview(&self, provider: &str, token: &str, org: &str, base: &str) -> Result<Value> {
        validate_scope(provider, org, base)?;
        match provider {
            "github" => {
                let value = self.json("https://api.github.com/graphql", &format!("Bearer {token}"), Some("query { viewer { login repositories(first:20, affiliations:[OWNER,COLLABORATOR,ORGANIZATION_MEMBER]) { nodes { nameWithOwner } pageInfo { hasNextPage } } } }"))?;
                let viewer = &value["data"]["viewer"];
                Ok(
                    json!({"identity":public_text(viewer,"login")?,"resources":names(&viewer["repositories"]["nodes"],"nameWithOwner")?,"resourceLabel":"Repositories","truncated":viewer["repositories"]["pageInfo"]["hasNextPage"]==true}),
                )
            }
            "linear" => {
                let value = self.json("https://api.linear.app/graphql", token, Some("query { viewer { id name } teams(first:20) { nodes { name } pageInfo { hasNextPage } } projects(first:20) { nodes { name } pageInfo { hasNextPage } } }"))?;
                let data = &value["data"];
                Ok(
                    json!({"identity":public_text(&data["viewer"],"name")?,"resources":names(&data["projects"]["nodes"],"name")?,"teams":names(&data["teams"]["nodes"],"name")?,"resourceLabel":"Projects","truncated":data["projects"]["pageInfo"]["hasNextPage"]==true || data["teams"]["pageInfo"]["hasNextPage"]==true}),
                )
            }
            "sentry" => {
                let projects = self.json(
                    &format!("{base}/api/0/organizations/{org}/projects/?per_page=20"),
                    &format!("Bearer {token}"),
                    None,
                )?;
                let issues = self.json(
                    &format!("{base}/api/0/organizations/{org}/issues/?limit=1&statsPeriod=1d"),
                    &format!("Bearer {token}"),
                    None,
                )?;
                if !issues.is_array() {
                    return Err(Error::new("Sentry did not verify issue read access."));
                }
                Ok(
                    json!({"identity":org,"resources":names(&projects,"slug")?,"resourceLabel":"Projects","truncated":projects.as_array().is_some_and(|rows|rows.len()>=20),"identityKind":"organization"}),
                )
            }
            _ => Err(Error::new("Unknown connection provider.")),
        }
    }
}
struct Pending {
    id: String,
    provider: String,
    token: Zeroizing<String>,
    org: String,
    base: String,
    identity: String,
    revision: u64,
    expires: f64,
}
pub struct SetupHost {
    profile: PathBuf,
    store: Arc<dyn CredentialStore>,
    remote: Arc<dyn Remote>,
    pending: Arc<Mutex<Option<Pending>>>,
    generation: AtomicU64,
}
impl SetupHost {
    pub fn new(profile: &Path) -> Result<Self> {
        Self::with_adapters(profile, Arc::new(Keychain), Arc::new(ProviderRemote::new()))
    }
    fn with_adapters(
        profile: &Path,
        store: Arc<dyn CredentialStore>,
        remote: Arc<dyn Remote>,
    ) -> Result<Self> {
        let profile = profile
            .canonicalize()
            .map_err(|_| Error::new("The active service profile is unavailable."))?;
        load_config(&profile)?;
        Ok(Self {
            profile,
            store,
            remote,
            pending: Arc::new(Mutex::new(None)),
            generation: AtomicU64::new(0),
        })
    }
    pub fn view(&self) -> Result<Value> {
        Ok(config_view(&load_config(&self.profile)?))
    }
    fn clear_pending(&self) -> u64 {
        let mut pending = self.pending.lock().unwrap();
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        pending.take();
        generation
    }
    pub fn cancel(&self) -> Value {
        self.clear_pending();
        json!({"cancelled":true})
    }
    pub fn begin_preview(&self) -> u64 {
        self.clear_pending()
    }
    fn check_ticket(&self, ticket: u64) -> Result<()> {
        if self.generation.load(Ordering::SeqCst) != ticket {
            return Err(Error::new("Connection preview was cancelled."));
        }
        Ok(())
    }
    pub fn preview_with_ticket(&self, payload: &Value, ticket: u64) -> Result<Value> {
        self.preview_with_source(payload, ticket, github_cli_token)
    }
    fn preview_with_source(
        &self,
        payload: &Value,
        ticket: u64,
        github_token: impl FnOnce() -> Result<Zeroizing<String>>,
    ) -> Result<Value> {
        check_fields(payload, &["provider", "token", "source", "org", "base"])?;
        self.check_ticket(ticket)?;
        let provider = text(payload, "provider");
        let org = text(payload, "org");
        let base = if provider == "sentry" && text(payload, "base").is_empty() {
            "https://sentry.io"
        } else {
            text(payload, "base")
        };
        validate_scope(provider, org, base)?;
        let config = load_config(&self.profile)?;
        self.check_ticket(ticket)?;
        let token = match text(payload, "source") {
            "manual" => secret(text(payload, "token"))?,
            "github-cli" if provider == "github" && text(payload, "token").is_empty() => {
                github_token()?
            }
            _ => {
                return Err(Error::new(
                    "Choose manual token entry or the GitHub CLI account.",
                ))
            }
        };
        self.check_ticket(ticket)?;
        let mut preview = self.remote.preview(provider, &token, org, base)?;
        if preview.to_string().contains(token.as_str()) {
            return Err(Error::new(
                "The provider returned an unsafe credential preview.",
            ));
        }
        let identity = public_text(&preview, "identity")?;
        let id = uuid();
        let mut pending = self.pending.lock().map_err(|_| Error::io())?;
        self.check_ticket(ticket)?;
        *pending = Some(Pending {
            id: id.clone(),
            provider: provider.into(),
            token,
            org: org.into(),
            base: base.into(),
            identity,
            revision: config.revision,
            expires: now() + TTL,
        });
        drop(pending);
        preview["previewId"] = json!(id);
        preview["expectedRevision"] = json!(config.revision);
        preview["expiresInSeconds"] = json!(TTL);
        preview["provider"] = json!(provider);
        let expiry = Arc::downgrade(&self.pending);
        let expiry_id = id;
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(300));
            if let Some(pending) = expiry.upgrade() {
                if let Ok(mut pending) = pending.lock() {
                    if pending.as_ref().is_some_and(|value| value.id == expiry_id) {
                        pending.take();
                    }
                }
            }
        });
        Ok(preview)
    }
    pub fn confirmation(&self, payload: &Value) -> Result<Value> {
        check_fields(payload, &["previewId", "expectedRevision"])?;
        let mut pending = self.pending.lock().map_err(|_| Error::io())?;
        let Some(current) = pending.as_ref() else {
            return Err(Error::new("Validate this connection again before saving."));
        };
        if current.expires <= now() {
            pending.take();
            return Err(Error::new(
                "The preview expired. Validate this connection again.",
            ));
        }
        if current.id != text(payload, "previewId")
            || payload["expectedRevision"].as_u64() != Some(current.revision)
            || load_config(&self.profile)?.revision != current.revision
        {
            return Err(Error::new(
                "Connections changed. Validate this connection again.",
            ));
        }
        let view =
            json!({"provider":current.provider,"identity":current.identity,"profile":self.profile});
        drop(pending);
        Ok(view)
    }
    pub fn commit(&self, payload: &Value) -> Result<Value> {
        self.commit_with_sync(payload, |directory| {
            std::fs::File::open(directory)?.sync_all()?;
            Ok(())
        })
    }
    fn commit_with_sync(
        &self,
        payload: &Value,
        sync_directory: impl FnOnce(&Path) -> Result<()>,
    ) -> Result<Value> {
        check_fields(payload, &["previewId", "expectedRevision"])?;
        let mut pending = self.pending.lock().map_err(|_| Error::io())?;
        let Some(current) = pending.as_ref() else {
            return Err(Error::new("Validate this connection again before saving."));
        };
        if current.expires <= now() {
            pending.take();
            return Err(Error::new(
                "The preview expired. Validate this connection again.",
            ));
        }
        if current.id != text(payload, "previewId")
            || payload["expectedRevision"].as_u64() != Some(current.revision)
        {
            return Err(Error::new(
                "The connection preview changed. Validate it again.",
            ));
        }
        let directory = self.profile.join("config");
        std::fs::create_dir_all(&directory)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(directory.join("connections.lock"))?;
        lock.lock()?;
        let mut config = load_config(&self.profile)?;
        if config.revision != current.revision {
            return Err(Error::new(
                "Connections changed in another window. Validate this connection again.",
            ));
        }
        let reference = uuid();
        let new_account = account(&self.profile, &reference);
        self.store.put(&new_account, &current.token)?;
        let old = config.providers.insert(
            current.provider.clone(),
            Entry {
                credential_ref: reference,
                identity: current.identity.clone(),
                org: current.org.clone(),
                base: current.base.clone(),
            },
        );
        config.revision += 1;
        let bytes = serde_json::to_vec_pretty(&config)?;
        let path = config_path(&self.profile);
        let temporary = directory.join(format!("connections-{}.tmp", uuid()));
        let mut renamed = false;
        let written = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            std::fs::rename(&temporary, &path)?;
            renamed = true;
            sync_directory(&directory)?;
            Ok(())
        })();
        if written.is_err() {
            if renamed {
                pending.take();
                return Err(Error::new("The connection save could not be confirmed durable. Keychain entries were retained. Reopen Connections before retrying."));
            }
            let _ = std::fs::remove_file(&temporary);
            let _ = self.store.delete(&new_account);
            return Err(Error::new("Connection configuration could not be saved. The previous connection remains active."));
        }
        pending.take();
        drop(pending);
        let mut view = config_view(&config);
        if let Some(old) = old {
            if self
                .store
                .delete(&account(&self.profile, &old.credential_ref))
                .is_err()
            {
                view["warning"] =
                    json!("Connection saved. An old unused Keychain entry could not be removed.");
            }
        }
        Ok(view)
    }
}
fn check_fields(payload: &Value, allowed: &[&str]) -> Result<()> {
    if payload
        .as_object()
        .is_none_or(|object| object.keys().any(|key| !allowed.contains(&key.as_str())))
    {
        return Err(Error::new("Unknown connection setup field."));
    }
    Ok(())
}
fn github_cli_token() -> Result<Zeroizing<String>> {
    let mut command = Command::new(super::terminal::executable("gh"));
    command
        .args(["auth", "token", "--hostname", "github.com", "--no-refresh"])
        .env_clear();
    for key in ["HOME", "PATH", "GH_CONFIG_DIR", "XDG_CONFIG_HOME"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .env("GH_PROMPT_DISABLED", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|_| Error::new("GitHub CLI could not start. Paste a token or install gh."))?;
    let output = child.stdout.take().ok_or_else(Error::io)?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Zeroizing::new(Vec::new());
        output
            .take((MAX_SECRET + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let Some(status) = super::terminal::wait_timeout(&mut child, Duration::from_secs(15))? else {
        let _ = child.kill();
        let _ = child.wait();
        let _ = reader.join();
        return Err(Error::new("GitHub CLI token lookup timed out."));
    };
    let bytes = reader.join().map_err(|_| Error::io())??;
    if !status.success() || bytes.len() > MAX_SECRET {
        return Err(Error::new("GitHub CLI has no usable github.com login. Sign in with gh auth login or paste a token."));
    }
    secret(
        std::str::from_utf8(&bytes)
            .map_err(|_| Error::new("GitHub CLI returned an invalid token."))?,
    )
}

struct ProviderSnapshot {
    provider: super::providers::Providers,
    errors: BTreeMap<String, String>,
}
pub struct ProviderHost {
    root: PathBuf,
    profile: PathBuf,
    store: Arc<dyn CredentialStore>,
    current: RwLock<ProviderSnapshot>,
    reload_guard: Mutex<()>,
}
impl ProviderHost {
    pub fn new(root: &Path, profile: &Path) -> Result<Self> {
        Self::with_store(root, profile, Arc::new(Keychain))
    }
    fn with_store(root: &Path, profile: &Path, store: Arc<dyn CredentialStore>) -> Result<Self> {
        let profile = profile.canonicalize()?;
        let provider = super::providers::Providers::new(root)?;
        let host = Self {
            root: root.into(),
            profile,
            store,
            reload_guard: Mutex::new(()),
            current: RwLock::new(ProviderSnapshot {
                provider,
                errors: BTreeMap::new(),
            }),
        };
        host.reload()?;
        Ok(host)
    }
    pub fn reload(&self) -> Result<Value> {
        let _reload = self.reload_guard.lock().map_err(|_| Error::io())?;
        let config = load_config(&self.profile)?;
        let mut provider = super::providers::Providers::new(&self.root)?;
        let mut errors = BTreeMap::new();
        for (source, entry) in &config.providers {
            match self
                .store
                .get(&account(&self.profile, &entry.credential_ref))
            {
                Ok(token) => {
                    provider.configure_connection(source, &token, &entry.org, &entry.base)?;
                }
                Err(error) => {
                    provider.configure_connection(source, "", &entry.org, &entry.base)?;
                    errors.insert(source.clone(), error.message);
                }
            }
        }
        let mut current = self.current.write().map_err(|_| Error::io())?;
        provider.retain_redactions(&current.provider);
        *current = ProviderSnapshot { provider, errors };
        drop(current);
        Ok(self.metadata())
    }
    fn snapshot(&self, source: &str) -> Result<super::providers::Providers> {
        let current = self.current.read().map_err(|_| Error::io())?;
        if let Some(error) = current.errors.get(source) {
            return Err(Error::new(error.clone()));
        }
        Ok(current.provider.clone())
    }
    pub fn metadata(&self) -> Value {
        let current = self.current.read().unwrap();
        json!({"configured":current.provider.sentry_configured()&&!current.errors.contains_key("sentry"),"org":current.provider.sentry_org,"period":current.provider.sentry_period,"errors":current.errors})
    }
    pub fn sentry_configured(&self) -> bool {
        self.metadata()["configured"] == true
    }
    pub(super) fn stream_redactor(&self) -> super::providers::StreamRedactor {
        self.current.read().unwrap().provider.stream_redactor()
    }
    pub fn redact_text(&self, value: &mut String) {
        self.current.read().unwrap().provider.redact_text(value);
    }
    pub fn refresh(&self, source: &str, refs: &[Value]) -> Result<Vec<Value>> {
        self.snapshot(source)?.refresh(source, refs)
    }
    pub fn resolve(&self, url: &str) -> Result<Value> {
        let parsed = url::Url::parse(url).map_err(|_| Error::new("Invalid provider URL."))?;
        let host = parsed.host_str().unwrap_or("");
        let source = if host == "linear.app" {
            "linear"
        } else if host == "sentry.io" || host.ends_with(".sentry.io") {
            "sentry"
        } else {
            "github"
        };
        self.snapshot(source)?.resolve(url)
    }
    pub fn linear_url(&self, id: &str) -> Result<String> {
        self.snapshot("linear")?.linear_url(id)
    }
    pub fn github_pr_action(&self, id: &str, action: &str, login: &str) -> Result<(Value, String)> {
        self.snapshot("github")?.github_pr_action(id, action, login)
    }
}

impl ProviderSource for ProviderHost {
    fn fetch(&self, source: &str, environment: Option<&str>) -> Result<Vec<Value>> {
        self.snapshot(source)?.fetch(source, environment)
    }
    fn refresh(&self, source: &str, refs: &[Value]) -> Result<Vec<Value>> {
        Self::refresh(self, source, refs)
    }
}

pub fn status(repo: &Repository) -> Result<Value> {
    Ok(config_view(&load_config(
        repo.data.parent().ok_or_else(Error::io)?,
    )?))
}

pub fn reload(repo: &Arc<Repository>, payload: &Value) -> Result<Value> {
    check_fields(payload, &["provider", "activate"])?;
    let provider = text(payload, "provider");
    if !PROVIDERS.contains(&provider)
        || payload
            .get("activate")
            .is_some_and(|value| !value.is_boolean())
    {
        return Err(Error::new(
            "Choose a supported connection provider and activation flag.",
        ));
    }
    let config = load_config(&repo.providers.profile)?;
    if payload["activate"] == true && !config.providers.contains_key(provider) {
        return Err(Error::new(
            "Validate and save this connection before importing work items.",
        ));
    }
    let result = repo.providers.reload()?;
    repo.invalidate_snapshot();
    if payload["activate"] == true {
        if result["errors"][provider].is_string() {
            return Err(Error::new(
                "This connection could not be loaded. Validate and save it again.",
            ));
        }
        repo.transaction(|state| {
            state["sources"][provider]["disabled"] = json!(false);
            state["sources"][provider]["nextAttempt"] = json!(0);
            Ok(())
        })?;
        super::source_sync::request(repo, false)?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
