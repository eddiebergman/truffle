use super::*;

fn sources_fixture() -> (tempfile::TempDir, Arc<Repository>) {
    let temp = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(temp.path().into(), None, true).unwrap();
    Arc::get_mut(&mut repo).unwrap().demo = false;
    (temp, repo)
}
#[test]
fn preparation_aliases_stay_private_in_dashboard_and_captured_open_snapshots() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let id = uuid();
    repo.transaction(|state|{state["sessions"]=json!([{"id":id,"agent":"codex","title":"Projection fixture","cwd":temp.path(),"items":[],"preparationRequests":{"version":1,"entries":[],"privateNs":"private-preparation-canary"}}]);Ok(())}).unwrap();
    let public = repo.snapshot(false);
    assert!(public["sessions"][0].get("preparationRequests").is_none());
    let (raw, captured) = repo.capture_for_open().unwrap();
    assert!(!captured.to_string().contains("private-preparation-canary"));
    assert_eq!(
        raw["sessions"][0]["preparationRequests"]["privateNs"],
        "private-preparation-canary"
    );
    assert_eq!(
        repo.snapshot(true)["sessions"][0]["preparationRequests"],
        raw["sessions"][0]["preparationRequests"]
    );
    assert_eq!(
        repo.state.lock().unwrap()["sessions"][0]["preparationRequests"],
        raw["sessions"][0]["preparationRequests"]
    );
}

#[test]
fn conditional_transactions_preserve_legacy_always_commit_and_skip_only_equal_states() {
    let (_temp, repo) = sources_fixture();
    let initial = repo.snapshot(false)["revision"].clone();
    repo.transaction(|_| Ok(())).unwrap();
    let legacy = repo.snapshot(false)["revision"].clone();
    assert_ne!(legacy, initial);
    repo.transaction_if_changed(|_| Ok(())).unwrap();
    assert_eq!(repo.snapshot(false)["revision"], legacy);
    repo.transaction_if_changed(|s| {
        s["conditionalFixture"] = json!(true);
        Ok(())
    })
    .unwrap();
    assert_ne!(repo.snapshot(false)["revision"], legacy);
    let disk: Value = serde_json::from_slice(&std::fs::read(&repo.data).unwrap()).unwrap();
    assert_eq!(disk["conditionalFixture"], true);
}

#[test]
fn presentation_requests_use_owner_reads_instead_of_public_snapshots() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state| {
        state["uiPresentations"] =
            json!({"version":1,"entries":[],"privateExtension":"private-presentation-canary"});
        Ok(())
    })
    .unwrap();
    for snapshot in [
        repo.snapshot(false),
        repo.snapshot(true),
        repo.capture_for_open().unwrap().1,
    ] {
        assert!(snapshot.get("uiPresentations").is_none());
        assert!(!snapshot.to_string().contains("private-presentation-canary"));
    }
    assert_eq!(
        repo.state.lock().unwrap()["uiPresentations"]["privateExtension"],
        "private-presentation-canary"
    );
}
#[test]
fn repository_drop_releases_lock_even_with_an_inherited_descriptor() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    // try_clone has the same shared open-file description as fork inheritance.
    let Repository {
        _lock: DataFileLock(lock_file),
        ..
    } = &*repo;
    let inherited = lock_file.try_clone().unwrap();
    assert!(Repository::open(temp.path().into(), None, true).is_err());
    drop(repo);
    let reopened = Repository::open(temp.path().into(), None, true).unwrap();
    drop(inherited);
    // Closing the old descriptor must not unlock the new repository's lock.
    assert!(Repository::open(temp.path().into(), None, true).is_err());
    drop(reopened);
    assert!(Repository::open(temp.path().into(), None, true).is_ok());
}

#[test]
fn data_lock_releases_on_constructor_error_with_a_duplicate_still_open() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state.lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let inherited = file.try_clone().unwrap();
    let fail_after_acquiring = || -> Result<()> {
        let _lock = DataFileLock::acquire(file)?;
        Err(Error::new(
            "Fixture constructor failure after acquiring the lock.",
        ))
    };
    assert!(fail_after_acquiring().is_err());
    let contender = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let lock = DataFileLock::acquire(contender).unwrap();
    drop(inherited);
    drop(lock);
}

#[test]
fn routine_snapshots_omit_review_bodies_without_changing_exports_or_history() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().to_path_buf(), None, true).unwrap();
    let record = repo
        .transaction(|state| {
            let record = super::super::sessions::save(
                state,
                &json!({
                    "agent":"claude", "cwd":temp.path(), "title":"Review snapshot", "itemIds":[]
                }),
            )?;
            let session = state["sessions"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|session| session["id"] == record["id"])
                .unwrap();
            session["reviewDraft"] = json!({"comments":[],"note":"Current review","revision":1});
            session["reviewHistory"] = json!({"version":1,"entries":[{
                "id":"previous", "archivedAt":1, "review":{"comments":[],"note":"Previous review"}
            }]});
            Ok(record)
        })
        .unwrap();

    for _ in 0..2 {
        let snapshot = repo.snapshot(false);
        let session = &snapshot["sessions"][0];
        assert!(session.get("reviewDraft").is_none());
        assert!(session.get("reviewHistory").is_none());
        assert_eq!(session["title"], "Review snapshot");
    }
    let export = repo.snapshot(true);
    assert_eq!(
        export["sessions"][0]["reviewDraft"]["note"],
        "Current review"
    );
    assert_eq!(
        export["sessions"][0]["reviewHistory"]["entries"][0]["review"]["note"],
        "Previous review"
    );
    let history = super::super::reviews::action(
        &repo,
        &super::super::reviews::Git,
        &json!({"id":record["id"],"action":"history"}),
    )
    .unwrap();
    assert_eq!(
        history["reviews"],
        export["sessions"][0]["reviewHistory"]["entries"]
    );
}
