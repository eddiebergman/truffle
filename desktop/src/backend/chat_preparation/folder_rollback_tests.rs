use super::*;
use crate::backend::uuid;
use std::os::unix::fs::symlink;
use std::sync::Arc;
fn fixture() -> (tempfile::TempDir, Arc<Repository>, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().join("profile"), None, true).unwrap();
    repo.transaction(|_| Ok(())).unwrap();
    let root = temp.path().join("work/projects");
    (temp, repo, root)
}
fn args() -> Value {
    json!({"agent":"codex","newFolder":"owned-project"})
}
/// Save with the work directory at `root`.
fn save_in(root: &std::path::Path, repo: &Repository, kind: Kind, intent: &Value) -> Result<Value> {
    let _env = crate::backend::tests::ENV.lock().unwrap();
    let previous = std::env::var_os("TRUFFLE_WORKDIR");
    std::env::set_var("TRUFFLE_WORKDIR", root);
    let result = save(repo, kind, intent);
    if let Some(value) = previous {
        std::env::set_var("TRUFFLE_WORKDIR", value);
    } else {
        std::env::remove_var("TRUFFLE_WORKDIR");
    }
    result
}
#[test]
fn quick_folder_canonical_failure_removes_only_created_leaf_and_immediate_retry_succeeds() {
    let (_temp, repo, root) = fixture();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    repo.cooldown
        .lock()
        .unwrap()
        .insert("quick-session".into(), 1.0);
    let cooldown = repo.cooldown.lock().unwrap().clone();
    let blocked = repo.data.with_extension("tmp");
    std::fs::create_dir(&blocked).unwrap();
    let error = save_in(&root, &repo, Kind::Quick, &args()).unwrap_err();
    assert_eq!(error.status, 500);
    assert!(!error.message.contains("retained"));
    assert!(!root.join("owned-project").exists());
    assert!(root.is_dir());
    assert!(root.parent().unwrap().is_dir());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert_eq!(*repo.cooldown.lock().unwrap(), cooldown);
    std::fs::remove_dir(blocked).unwrap();
    let record = save_in(&root, &repo, Kind::Quick, &args()).unwrap();
    assert!(root.join("owned-project").is_dir());
    assert_eq!(
        record["cwd"],
        json!(root.join("owned-project").canonicalize().unwrap())
    );
    assert_eq!(repo.embedded.inventory(), [] as [serde_json::Value; 0]);
}
#[test]
fn quick_folder_review_validation_precedes_creation_for_missing_or_malformed_template() {
    for malformed in [false, true] {
        let (_temp, repo, root) = fixture();
        let settings = repo.settings.view();
        repo.settings.apply(&json!({"revision":settings["revision"],"contentHash":settings["contentHash"],"patch":{"codexReviewCommand":"$fixture-review"}}),false).unwrap();
        let template = repo
            .settings
            .config_directory()
            .join("workflows/pr-review-codex.md");
        if malformed {
            std::fs::write(&template, [0xff]).unwrap();
        } else {
            std::fs::remove_file(&template).unwrap();
        }
        repo.transaction(|s|{s["items"]=json!([{"id":"github:fixture","source":"github","kind":"Pull request","title":"Fixture","url":"https://github.com/example/project/pull/1","reference":"#1"}]);Ok(())}).unwrap();
        let mut payload = args();
        payload["itemId"] = json!("github:fixture");
        let before = std::fs::read(&repo.data).unwrap();
        let cooldown = repo.cooldown.lock().unwrap().clone();
        assert!(save_in(&root, &repo, Kind::PrReview, &payload).is_err());
        assert!(!root.exists());
        assert_eq!(std::fs::read(&repo.data).unwrap(), before);
        assert_eq!(*repo.cooldown.lock().unwrap(), cooldown);
    }
}
#[test]
fn quick_folder_invalid_title_or_instruction_is_rejected_before_creation() {
    for invalid_instruction in [false, true] {
        let (_temp, repo, root) = fixture();
        let mut payload = args();
        let review = invalid_instruction;
        if review {
            let settings = repo.settings.view();
            repo.settings.apply(&json!({"revision":settings["revision"],"contentHash":settings["contentHash"],"patch":{"codexReviewCommand":"$fixture-review"}}),false).unwrap();
            std::fs::write(
                repo.settings
                    .config_directory()
                    .join("workflows/pr-review-codex.md"),
                "x".repeat(8001),
            )
            .unwrap();
        }
        repo.transaction(|s|{s["items"]=json!([{"id":"github:fixture","source":"github","kind":"Pull request","title":"Invalid\nTitle","url":"https://github.com/example/project/pull/1"}]);Ok(())}).unwrap();
        payload["itemId"] = json!("github:fixture");
        let bytes = std::fs::read(&repo.data).unwrap();
        assert!(save_in(
            &root,
            &repo,
            if review { Kind::PrReview } else { Kind::Quick },
            &payload
        )
        .is_err());
        assert!(!root.exists());
        assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    }
}
#[test]
fn quick_folder_cleanup_preserves_added_content_and_original_error_details() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("owned");
    let mut folder = Some(Created::Folder(CreatedFolder::create(&path).unwrap()));
    std::fs::write(path.join("someone-else.txt"), b"keep content").unwrap();
    let mut failure = Error::new("Original failure.");
    failure.status = 409;
    failure.data = json!({"existing":"details"});
    failure.retry = 17.0;
    let error = rollback(Err(failure), &mut folder).unwrap_err();
    assert_eq!(error.status, 409);
    assert_eq!(error.data, json!({"existing":"details"}));
    assert_eq!(error.retry, 17.0);
    assert!(error.message.starts_with("Original failure."));
    assert!(error.message.contains("retained"));
    assert_eq!(
        std::fs::read(path.join("someone-else.txt")).unwrap(),
        b"keep content"
    );
    std::fs::remove_file(path.join("someone-else.txt")).unwrap();
    drop(folder);
    assert!(
        path.is_dir(),
        "Drop must not retry a cleanup that already chose preservation"
    );
}
#[test]
fn quick_folder_cleanup_preserves_replacement_inode_and_symlink_targets() {
    for replace_symlink in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("owned");
        let original = temp.path().join("renamed-owned");
        let mut folder = Some(Created::Folder(CreatedFolder::create(&path).unwrap()));
        std::fs::rename(&path, &original).unwrap();
        if replace_symlink {
            let target = temp.path().join("target");
            std::fs::create_dir(&target).unwrap();
            std::fs::write(target.join("keep"), b"target data").unwrap();
            symlink(&target, &path).unwrap();
        } else {
            std::fs::create_dir(&path).unwrap();
        }
        let error = rollback(Err(Error::new("Fixture failure")), &mut folder).unwrap_err();
        assert!(error.message.contains("retained"));
        drop(folder);
        assert!(original.is_dir());
        assert!(std::fs::symlink_metadata(&path).is_ok());
        if replace_symlink {
            assert!(std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(std::fs::read(path.join("keep")).unwrap(), b"target data");
        }
    }
}
#[test]
fn quick_folder_success_disarms_cleanup_and_preexisting_targets_are_never_owned() {
    let (_temp, repo, root) = fixture();
    std::fs::create_dir_all(&root).unwrap();
    let target = root.join("owned-project");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("keep"), b"preexisting").unwrap();
    let before = std::fs::read(&repo.data).unwrap();
    assert!(save_in(&root, &repo, Kind::Quick, &args()).is_err());
    assert_eq!(std::fs::read(target.join("keep")).unwrap(), b"preexisting");
    assert_eq!(std::fs::read(&repo.data).unwrap(), before);
    let path = root.join("committed");
    let mut folder = Some(Created::Folder(CreatedFolder::create(&path).unwrap()));
    let request = uuid();
    let record = json!({"id":uuid(),"cwd":path,"preparationRequests":{"version":1,"entries":[{"requestId":request,"kind":"quick","hash":context_kit::sha256(&serde_json::to_vec(&json!({"version":1,"kind":"quick","intent":args()})).unwrap()),"reused":false}]}});
    repo.transaction(|s| {
        s["sessions"].as_array_mut().unwrap().push(record.clone());
        Ok(())
    })
    .unwrap();
    assert_eq!(rollback(Ok(record.clone()), &mut folder).unwrap(), record);
    drop(folder);
    assert!(path.is_dir());
    let later_error = Error::new("Response projection failed");
    drop(later_error);
    assert!(path.is_dir());
    let expected = read(&repo, &json!({"requestId":request})).unwrap();
    let mut retry = args();
    retry["requestId"] = json!(request);
    let bytes = std::fs::read(&repo.data).unwrap();
    assert_eq!(prepare(&repo, Kind::Quick, &retry).unwrap(), expected);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert!(path.is_dir());
}
#[test]
fn quick_folder_missing_identity_is_retained_and_drop_only_removes_empty_owned_leaf() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("owned");
    let mut folder = CreatedFolder::create(&path).unwrap();
    folder.identity = None;
    let error = rollback(
        Err(Error::new("Cannot inspect identity")),
        &mut Some(Created::Folder(folder)),
    )
    .unwrap_err();
    assert!(error.message.contains("retained"));
    assert!(path.exists());
    let empty = temp.path().join("empty");
    {
        let _folder = CreatedFolder::create(&empty).unwrap();
    }
    assert!(!empty.exists());
    assert!(temp.path().exists());
}
