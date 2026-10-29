//! Native credential setup. Public projections contain no credentials.
use super::*;
use fs2::FileExt;
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
use wait_timeout::ChildExt;
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
    client: reqwest::blocking::Client,
}
impl ProviderRemote {
    fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::blocking::Client::builder()
                .user_agent("truffle-connections/1")
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(20))
                .build()
                .map_err(|_| Error::io())?,
        })
    }
    fn json(&self, url: &str, token: &str, query: Option<&str>) -> Result<Value> {
        let mut auth = reqwest::header::HeaderValue::from_str(token)
            .map_err(|_| Error::new("Invalid token format."))?;
        auth.set_sensitive(true);
        let request = match query {
            Some(query) => self.client.post(url).json(&json!({"query":query})),
            None => self.client.get(url),
        };
        let response = request
            .header(reqwest::header::AUTHORIZATION, auth)
            .send()
            .map_err(|_| {
                Error::new("Connection validation failed. Check your network and try again.")
            })?;
        if !response.status().is_success() {
            return Err(Error::new(format!("The provider returned HTTP {}. Check token permissions, expiration and organization approval.", response.status().as_u16())));
        }
        let mut bytes = Zeroizing::new(Vec::new());
        response.take(524_289).read_to_end(&mut bytes)?;
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
    let value = value[key].as_str().ok_or(Error::new(
        "The provider returned an incomplete identity or resource.",
    ))?;
    if value.is_empty() || value.len() > 200 || value.chars().any(char::is_control) {
        return Err(Error::new(
            "The provider returned an invalid identity or resource.",
        ));
    }
    Ok(value.into())
}
fn names(value: &Value, key: &str) -> Result<Vec<String>> {
    let rows = value.as_array().ok_or(Error::new(
        "The provider returned an invalid resource list.",
    ))?;
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
    pub fn new(profile: PathBuf) -> Result<Self> {
        Self::with_adapters(
            profile,
            Arc::new(Keychain),
            Arc::new(ProviderRemote::new()?),
        )
    }
    fn with_adapters(
        profile: PathBuf,
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
    pub fn preview(&self, payload: &Value) -> Result<Value> {
        check_fields(payload, &["provider", "token", "source", "org", "base"])?;
        self.preview_with_ticket(payload, self.begin_preview())
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
        preview["previewId"] = json!(id);
        preview["expectedRevision"] = json!(config.revision);
        preview["expiresInSeconds"] = json!(TTL);
        preview["provider"] = json!(provider);
        let expiry = Arc::downgrade(&self.pending);
        let expiry_id = id.clone();
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
        Ok(json!({"provider":current.provider,"identity":current.identity,"profile":self.profile}))
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
            .mode(0o600)
            .open(directory.join("connections.lock"))?;
        lock.lock_exclusive()?;
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
    let output = child.stdout.take().ok_or(Error::io())?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Zeroizing::new(Vec::new());
        output
            .take((MAX_SECRET + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let status = match child.wait_timeout(Duration::from_secs(15))? {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(Error::new("GitHub CLI token lookup timed out."));
        }
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
                    provider.configure_connection(source, &token, &entry.org, &entry.base)?
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
    pub fn fetch(&self, source: &str, environment: Option<&str>) -> Result<Vec<Value>> {
        self.snapshot(source)?.fetch(source, environment)
    }
    pub fn refresh(&self, source: &str, refs: &[Value]) -> Result<Vec<Value>> {
        self.snapshot(source)?.refresh(source, refs)
    }
    pub fn resolve(&self, url: &str) -> Result<Value> {
        let parsed = reqwest::Url::parse(url).map_err(|_| Error::new("Invalid provider URL."))?;
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
    pub fn github_pr(&self, id: &str) -> Result<(Value, String)> {
        self.snapshot("github")?.github_pr(id)
    }
    pub fn github_pr_action(&self, id: &str, action: &str, login: &str) -> Result<(Value, String)> {
        self.snapshot("github")?.github_pr_action(id, action, login)
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
        repo.schedule(false)?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct MemoryStore {
        values: Mutex<BTreeMap<String, String>>,
        gets: AtomicU64,
        fail_get: Mutex<bool>,
        fail_put: Mutex<bool>,
    }
    impl CredentialStore for MemoryStore {
        fn get(&self, account: &str) -> Result<Zeroizing<String>> {
            self.gets.fetch_add(1, Ordering::SeqCst);
            if *self.fail_get.lock().unwrap() {
                return Err(Error::new("Fixture Keychain is locked."));
            }
            secret(
                self.values
                    .lock()
                    .unwrap()
                    .get(account)
                    .ok_or(Error::new("Fixture credential missing."))?,
            )
        }
        fn put(&self, account: &str, token: &str) -> Result<()> {
            if *self.fail_put.lock().unwrap() {
                return Err(Error::new("Fixture Keychain write denied."));
            }
            self.values
                .lock()
                .unwrap()
                .insert(account.into(), token.into());
            Ok(())
        }
        fn delete(&self, account: &str) -> Result<()> {
            self.values.lock().unwrap().remove(account);
            Ok(())
        }
    }
    struct FixtureRemote {
        response: Mutex<Value>,
        calls: AtomicU64,
    }
    impl Default for FixtureRemote {
        fn default() -> Self {
            Self {
                response: Mutex::new(
                    json!({"identity":"fixture-user","resources":["fixture/project"],"resourceLabel":"Repositories"}),
                ),
                calls: AtomicU64::new(0),
            }
        }
    }
    impl Remote for FixtureRemote {
        fn preview(&self, _provider: &str, _token: &str, _org: &str, _base: &str) -> Result<Value> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.response.lock().unwrap().clone())
        }
    }
    fn host(profile: &Path, store: Arc<MemoryStore>, remote: Arc<FixtureRemote>) -> SetupHost {
        SetupHost::with_adapters(profile.into(), store, remote).unwrap()
    }
    fn preview(host: &SetupHost) -> Value {
        host.preview(
            &json!({"provider":"github","source":"manual","token":"benign-fixture-secret"}),
        )
        .unwrap()
    }
    fn commit_payload(preview: &Value) -> Value {
        json!({"previewId":preview["previewId"],"expectedRevision":preview["expectedRevision"]})
    }
    #[test]
    fn preview_commit_and_rotation_keep_secrets_out_of_configuration_and_results() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::default());
        let broker = host(
            directory.path(),
            store.clone(),
            Arc::new(FixtureRemote::default()),
        );
        let first = preview(&broker);
        assert!(!config_path(directory.path()).exists());
        assert!(store.values.lock().unwrap().is_empty());
        assert!(!first.to_string().contains("benign-fixture-secret"));
        let confirmation = broker.confirmation(&commit_payload(&first)).unwrap();
        assert_eq!(confirmation["identity"], "fixture-user");
        assert!(!confirmation.to_string().contains("benign-fixture-secret"));
        let saved = broker.commit(&commit_payload(&first)).unwrap();
        assert_eq!(saved["revision"], 1);
        assert!(!saved.to_string().contains("benign-fixture-secret"));
        let bytes = std::fs::read_to_string(config_path(directory.path())).unwrap();
        assert!(!bytes.contains("benign-fixture-secret"));
        assert!(!bytes.contains("token"));
        assert_eq!(
            serde_json::from_str::<Value>(&bytes).unwrap()["storage"],
            STORAGE
        );
        assert_eq!(saved["storage"], STORAGE);
        assert!(broker.pending.lock().unwrap().is_none());
        assert_eq!(store.values.lock().unwrap().len(), 1);
        let second = preview(&broker);
        broker.commit(&commit_payload(&second)).unwrap();
        assert_eq!(store.values.lock().unwrap().len(), 1);
        assert_eq!(broker.view().unwrap()["revision"], 2);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(config_path(directory.path()))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    #[test]
    fn cancel_expiry_and_replaced_previews_cannot_commit() {
        let directory = tempfile::tempdir().unwrap();
        let broker = host(
            directory.path(),
            Arc::new(MemoryStore::default()),
            Arc::new(FixtureRemote::default()),
        );
        let first = preview(&broker);
        broker.cancel();
        assert!(broker.commit(&commit_payload(&first)).is_err());
        assert!(broker.pending.lock().unwrap().is_none());
        let second = preview(&broker);
        broker.pending.lock().unwrap().as_mut().unwrap().expires = now() - 1.0;
        assert!(broker.confirmation(&commit_payload(&second)).is_err());
        assert!(broker.pending.lock().unwrap().is_none());
        let old = preview(&broker);
        let current = preview(&broker);
        assert!(broker.commit(&commit_payload(&old)).is_err());
        assert!(broker.commit(&commit_payload(&current)).is_ok());
    }
    #[test]
    fn concurrent_profile_writes_use_revision_checks_before_storing_a_secret() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::default());
        let remote = Arc::new(FixtureRemote::default());
        let first = host(directory.path(), store.clone(), remote.clone());
        let second = host(directory.path(), store.clone(), remote);
        let a = preview(&first);
        let b = preview(&second);
        first.commit(&commit_payload(&a)).unwrap();
        assert!(second.confirmation(&commit_payload(&b)).is_err());
        assert!(second.commit(&commit_payload(&b)).is_err());
        assert_eq!(store.values.lock().unwrap().len(), 1);
    }
    #[test]
    fn denied_keychain_write_leaves_previous_configuration_intact() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::default());
        let broker = host(
            directory.path(),
            store.clone(),
            Arc::new(FixtureRemote::default()),
        );
        let first = preview(&broker);
        broker.commit(&commit_payload(&first)).unwrap();
        let before = std::fs::read(config_path(directory.path())).unwrap();
        let next = preview(&broker);
        *store.fail_put.lock().unwrap() = true;
        assert!(broker.commit(&commit_payload(&next)).is_err());
        assert_eq!(
            std::fs::read(config_path(directory.path())).unwrap(),
            before
        );
        assert_eq!(store.values.lock().unwrap().len(), 1);
    }
    #[test]
    fn profile_references_are_isolated_and_new_profiles_never_probe_keychain() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::default());
        let first = host(a.path(), store.clone(), Arc::new(FixtureRemote::default()));
        let second = host(b.path(), store.clone(), Arc::new(FixtureRemote::default()));
        assert_eq!(store.gets.load(Ordering::SeqCst), 0);
        let preview = preview(&first);
        first.commit(&commit_payload(&preview)).unwrap();
        assert_eq!(second.view().unwrap()["revision"], 0);
        assert_ne!(
            account(a.path(), "same-reference"),
            account(b.path(), "same-reference")
        );
        let provider = ProviderHost::with_store(b.path(), b.path(), store.clone()).unwrap();
        assert_eq!(store.gets.load(Ordering::SeqCst), 0);
        assert!(provider.metadata()["errors"]
            .as_object()
            .unwrap()
            .is_empty());
    }
    #[test]
    fn configured_keychain_failure_blocks_that_provider_without_identity_fallback() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::default());
        let broker = host(
            directory.path(),
            store.clone(),
            Arc::new(FixtureRemote::default()),
        );
        let first = preview(&broker);
        broker.commit(&commit_payload(&first)).unwrap();
        std::fs::write(
            directory.path().join(".env"),
            "GITHUB_TOKEN=benign-old-account",
        )
        .unwrap();
        *store.fail_get.lock().unwrap() = true;
        let provider = ProviderHost::with_store(directory.path(), directory.path(), store).unwrap();
        assert!(provider.snapshot("github").is_err());
        assert!(provider.snapshot("linear").is_ok());
        assert_eq!(
            provider.metadata()["errors"]["github"],
            "Fixture Keychain is locked."
        );
    }
    #[test]
    fn unsupported_config_and_unsafe_hosts_are_rejected_without_network_or_secret_writes() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::default());
        let remote = Arc::new(FixtureRemote::default());
        let broker = host(directory.path(), store.clone(), remote.clone());
        for payload in [
            json!({"provider":"sentry","source":"manual","token":"benign-fixture","org":"fixture","base":"https://attacker.invalid"}),
            json!({"provider":"github","source":"manual","token":"benign-fixture","base":"https://attacker.invalid"}),
            json!({"provider":"github","source":"manual","token":"benign-fixture","command":"anything"}),
        ] {
            assert!(broker.preview(&payload).is_err());
        }
        assert_eq!(remote.calls.load(Ordering::SeqCst), 0);
        assert!(store.values.lock().unwrap().is_empty());
        std::fs::create_dir_all(directory.path().join("config")).unwrap();
        std::fs::write(
            config_path(directory.path()),
            "{\"version\":2,\"revision\":1,\"providers\":{}}",
        )
        .unwrap();
        assert!(broker.view().is_err());
        assert!(broker
            .preview(&json!({"provider":"github","source":"manual","token":"benign-fixture"}))
            .is_err());
        assert_eq!(remote.calls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn provider_response_cannot_echo_the_secret_into_the_preview() {
        let directory = tempfile::tempdir().unwrap();
        let remote = Arc::new(FixtureRemote::default());
        *remote.response.lock().unwrap() =
            json!({"identity":"benign-fixture-secret","resources":[]});
        let broker = host(directory.path(), Arc::new(MemoryStore::default()), remote);
        let error = broker
            .preview(
                &json!({"provider":"github","source":"manual","token":"benign-fixture-secret"}),
            )
            .unwrap_err();
        assert!(!error.message.contains("benign-fixture-secret"));
        assert!(broker.pending.lock().unwrap().is_none());
    }
    #[test]
    fn remote_http_errors_redirects_and_graphql_errors_never_return_response_bodies() {
        let remote = ProviderRemote::new().unwrap();
        for (status, body) in [
            (401, "benign-provider-echo"),
            (302, "benign-provider-echo"),
            (200, "{\"errors\":[{\"message\":\"benign-provider-echo\"}]}"),
            (200, "not JSON benign-provider-echo"),
        ] {
            let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
            let address = format!("http://{}", server.server_addr());
            let responder = std::thread::spawn(move || {
                let request = server.recv().unwrap();
                request
                    .respond(tiny_http::Response::from_string(body).with_status_code(status))
                    .unwrap();
            });
            let error = remote
                .json(
                    &address,
                    "benign-fixture",
                    Some("query { viewer { login } }"),
                )
                .unwrap_err();
            assert!(!error.message.contains("benign-provider-echo"));
            responder.join().unwrap();
        }
    }
    #[test]
    fn uncertain_directory_durability_retains_old_and_new_keychain_entries() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::default());
        let broker = host(
            directory.path(),
            store.clone(),
            Arc::new(FixtureRemote::default()),
        );
        let first = preview(&broker);
        broker.commit(&commit_payload(&first)).unwrap();
        let old = load_config(directory.path()).unwrap().providers["github"]
            .credential_ref
            .clone();
        let next = preview(&broker);
        assert!(broker
            .commit_with_sync(&commit_payload(&next), |_| Err(Error::new(
                "Fixture directory sync failed."
            )))
            .is_err());
        assert_eq!(store.values.lock().unwrap().len(), 2);
        assert!(store
            .values
            .lock()
            .unwrap()
            .contains_key(&account(&broker.profile, &old)));
        assert!(broker.pending.lock().unwrap().is_none());
    }
    #[test]
    fn cancelled_in_flight_preview_cannot_replace_a_newer_account() {
        use std::sync::mpsc;
        struct OrderedRemote {
            calls: AtomicU64,
            started: mpsc::SyncSender<()>,
            release: Mutex<mpsc::Receiver<()>>,
        }
        impl Remote for OrderedRemote {
            fn preview(&self, _: &str, _: &str, _: &str, _: &str) -> Result<Value> {
                let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
                if first {
                    self.started.send(()).unwrap();
                    self.release.lock().unwrap().recv().unwrap();
                }
                Ok(json!({"identity":if first {"old-account"} else {"new-account"},"resources":[]}))
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::channel();
        let broker = Arc::new(
            SetupHost::with_adapters(
                directory.path().into(),
                Arc::new(MemoryStore::default()),
                Arc::new(OrderedRemote {
                    calls: AtomicU64::new(0),
                    started: started_tx,
                    release: Mutex::new(release_rx),
                }),
            )
            .unwrap(),
        );
        let old_broker = broker.clone();
        let old = std::thread::spawn(move || {
            old_broker.preview(
                &json!({"provider":"github","source":"manual","token":"benign-old-fixture"}),
            )
        });
        started_rx.recv().unwrap();
        let current = preview(&broker);
        release_tx.send(()).unwrap();
        assert!(old.join().unwrap().is_err());
        assert_eq!(
            broker.confirmation(&commit_payload(&current)).unwrap()["identity"],
            "new-account"
        );
    }
    #[test]
    fn provider_reload_serializes_keychain_reads_and_keeps_the_latest_account() {
        use std::sync::{atomic::AtomicBool, mpsc};
        struct BlockingStore {
            inner: MemoryStore,
            block: AtomicBool,
            started: mpsc::SyncSender<()>,
            release: Mutex<mpsc::Receiver<()>>,
        }
        impl CredentialStore for BlockingStore {
            fn get(&self, account: &str) -> Result<Zeroizing<String>> {
                let token = self.inner.get(account)?;
                if self.block.swap(false, Ordering::SeqCst) {
                    self.started.send(()).unwrap();
                    self.release.lock().unwrap().recv().unwrap();
                }
                Ok(token)
            }
            fn put(&self, account: &str, token: &str) -> Result<()> {
                self.inner.put(account, token)
            }
            fn delete(&self, account: &str) -> Result<()> {
                self.inner.delete(account)
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::channel();
        let store = Arc::new(BlockingStore {
            inner: MemoryStore::default(),
            block: AtomicBool::new(true),
            started: started_tx,
            release: Mutex::new(release_rx),
        });
        let providers = Arc::new(
            ProviderHost::with_store(directory.path(), directory.path(), store.clone()).unwrap(),
        );
        let broker = SetupHost::with_adapters(
            directory.path().into(),
            store,
            Arc::new(FixtureRemote::default()),
        )
        .unwrap();
        let first = broker.preview(&json!({"provider":"sentry","source":"manual","token":"benign-first-fixture","org":"first-org","base":"https://sentry.io"})).unwrap();
        broker.commit(&commit_payload(&first)).unwrap();
        let old_providers = providers.clone();
        let old = std::thread::spawn(move || old_providers.reload());
        started_rx.recv().unwrap();
        let second = broker.preview(&json!({"provider":"sentry","source":"manual","token":"benign-second-fixture","org":"second-org","base":"https://sentry.io"})).unwrap();
        broker.commit(&commit_payload(&second)).unwrap();
        let new_providers = providers.clone();
        let (done_tx, done_rx) = mpsc::channel();
        let new = std::thread::spawn(move || {
            let result = new_providers.reload();
            done_tx.send(()).unwrap();
            result
        });
        assert!(done_rx.recv_timeout(Duration::from_millis(50)).is_err());
        release_tx.send(()).unwrap();
        old.join().unwrap().unwrap();
        new.join().unwrap().unwrap();
        assert_eq!(providers.metadata()["org"], "second-org");
        assert!(providers.metadata()["errors"]
            .as_object()
            .unwrap()
            .is_empty());
    }
    #[test]
    fn connection_storage_backend_is_explicit_and_never_falls_back() {
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::default());
        let broker = host(
            directory.path(),
            store.clone(),
            Arc::new(FixtureRemote::default()),
        );
        let first = preview(&broker);
        broker.commit(&commit_payload(&first)).unwrap();
        let path = config_path(directory.path());
        let mut config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        config["storage"] = json!("other-keychain");
        let unsupported = serde_json::to_vec(&config).unwrap();
        std::fs::write(&path, &unsupported).unwrap();
        assert!(broker.view().is_err());
        assert!(broker
            .preview(&json!({"provider":"github","source":"manual","token":"benign-fixture"}))
            .is_err());
        assert_eq!(store.gets.load(Ordering::SeqCst), 0);
        assert_eq!(std::fs::read(path).unwrap(), unsupported);
    }
    #[test]
    fn cancelled_native_approval_ticket_never_reads_github_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let remote = Arc::new(FixtureRemote::default());
        let broker = host(
            directory.path(),
            Arc::new(MemoryStore::default()),
            remote.clone(),
        );
        let reads = AtomicU64::new(0);
        let ticket = broker.begin_preview();
        broker.cancel();
        let payload = json!({"provider":"github","source":"github-cli"});
        assert!(broker
            .preview_with_source(&payload, ticket, || {
                reads.fetch_add(1, Ordering::SeqCst);
                secret("benign-gh-fixture")
            })
            .is_err());
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        assert_eq!(remote.calls.load(Ordering::SeqCst), 0);
        assert!(broker.pending.lock().unwrap().is_none());
    }
    #[test]
    fn replaced_approval_tickets_stay_cancelled_and_newest_ticket_can_complete() {
        let directory = tempfile::tempdir().unwrap();
        let remote = Arc::new(FixtureRemote::default());
        let broker = host(
            directory.path(),
            Arc::new(MemoryStore::default()),
            remote.clone(),
        );
        let reads = AtomicU64::new(0);
        let previous = broker.begin_preview();
        let current = broker.begin_preview();
        let payload = json!({"provider":"github","source":"github-cli"});
        assert!(broker
            .preview_with_source(&payload, previous, || {
                reads.fetch_add(1, Ordering::SeqCst);
                secret("benign-gh-fixture")
            })
            .is_err());
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        let preview = broker
            .preview_with_source(&payload, current, || {
                reads.fetch_add(1, Ordering::SeqCst);
                secret("benign-gh-fixture")
            })
            .unwrap();
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert_eq!(remote.calls.load(Ordering::SeqCst), 1);
        assert!(broker.confirmation(&commit_payload(&preview)).is_ok());
    }
    #[test]
    fn cancellation_during_credential_read_prevents_provider_network_validation() {
        let directory = tempfile::tempdir().unwrap();
        let remote = Arc::new(FixtureRemote::default());
        let broker = host(
            directory.path(),
            Arc::new(MemoryStore::default()),
            remote.clone(),
        );
        let ticket = broker.begin_preview();
        assert!(broker
            .preview_with_source(
                &json!({"provider":"github","source":"github-cli"}),
                ticket,
                || {
                    broker.cancel();
                    secret("benign-gh-fixture")
                }
            )
            .is_err());
        assert_eq!(remote.calls.load(Ordering::SeqCst), 0);
        assert!(broker.pending.lock().unwrap().is_none());
    }
}
