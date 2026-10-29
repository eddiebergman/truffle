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
                .ok_or_else(|| Error::new("Fixture credential missing."))?,
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
fn preview_payload(host: &SetupHost, payload: &Value) -> Result<Value> {
    host.preview_with_ticket(payload, host.begin_preview())
}
fn preview(host: &SetupHost) -> Value {
    preview_payload(
        host,
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
        assert!(preview_payload(&broker, &payload).is_err());
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
    assert!(preview_payload(
        &broker,
        &json!({"provider":"github","source":"manual","token":"benign-fixture"})
    )
    .is_err());
    assert_eq!(remote.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn provider_response_cannot_echo_the_secret_into_the_preview() {
    let directory = tempfile::tempdir().unwrap();
    let remote = Arc::new(FixtureRemote::default());
    *remote.response.lock().unwrap() = json!({"identity":"benign-fixture-secret","resources":[]});
    let broker = host(directory.path(), Arc::new(MemoryStore::default()), remote);
    let error = preview_payload(
        &broker,
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
        preview_payload(
            &old_broker,
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
    let first = preview_payload(&broker, &json!({"provider":"sentry","source":"manual","token":"benign-first-fixture","org":"first-org","base":"https://sentry.io"})).unwrap();
    broker.commit(&commit_payload(&first)).unwrap();
    let old_providers = providers.clone();
    let old = std::thread::spawn(move || old_providers.reload());
    started_rx.recv().unwrap();
    let second = preview_payload(&broker, &json!({"provider":"sentry","source":"manual","token":"benign-second-fixture","org":"second-org","base":"https://sentry.io"})).unwrap();
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
    assert!(preview_payload(
        &broker,
        &json!({"provider":"github","source":"manual","token":"benign-fixture"})
    )
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
