//! Preparation saves context; it never opens a terminal.
//! Aliases last while their saved record is retained; absence does not prove no effect.
use super::*;
const MAX_ALIASES: usize = 128;
const RECEIPT: &str = "preparationReceipt";
pub(super) const KINDS: [&str; 8] = [
    "quick",
    "pr-review",
    "review",
    "skill",
    "recover",
    "new",
    "attach",
    "view",
];

pub(super) struct Request {
    id: String,
    pub kind: String,
    hash: String,
}
fn conflict(message: &str) -> Error {
    let mut error = Error::new(message);
    error.status = 409;
    error
}
fn exact(repo: &Repository, value: &str) -> Result<()> {
    let mut safe = value.to_owned();
    super::commands::redact_text(repo, &mut safe);
    if safe != value || value.chars().any(char::is_control) {
        return Err(Error::new(
            "Preparation identity cannot be safely projected.",
        ));
    }
    Ok(())
}
fn record_identity(repo: &Repository, record: &Value) -> Result<()> {
    let id = text(record, "id");
    if sid(id).ok().as_deref() != Some(id) {
        return Err(conflict("The saved preparation chat identity is invalid."));
    }
    exact(repo, id)
}
fn projection(repo: &Repository, id: &str) -> Result<()> {
    for value in [
        id,
        "prepared",
        "not-found",
        "quick",
        "pr-review",
        "review",
        "skill",
        "recover",
        "new",
        "attach",
        "view",
    ] {
        exact(repo, value)?;
    }
    Ok(())
}
fn aliases(record: &Value) -> Result<Vec<Value>> {
    let Some(document) = record.get("preparationRequests") else {
        return Ok(vec![]);
    };
    if document["version"] != 1 || serde_json::to_vec(document)?.len() > 64 * 1024 {
        return Err(conflict(
            "The saved preparation identities are unsupported or exceed their limit.",
        ));
    }
    let entries = document["entries"]
        .as_array()
        .filter(|rows| rows.len() <= MAX_ALIASES)
        .ok_or_else(|| conflict("The saved preparation identities are invalid."))?;
    let mut ids = std::collections::BTreeSet::new();
    for entry in entries {
        let id = sid(text(entry, "requestId"))
            .map_err(|_| conflict("The saved preparation identities are invalid."))?;
        if id != text(entry, "requestId")
            || !ids.insert(id)
            || !KINDS.contains(&text(entry, "kind"))
            || !entry["hash"].as_str().is_some_and(|hash| {
                hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            || !entry["reused"].is_boolean()
        {
            return Err(conflict("The saved preparation identities are invalid."));
        }
    }
    Ok(entries.clone())
}
fn find(state: &Value, id: &str) -> Result<Option<(Value, Value)>> {
    let mut seen = std::collections::BTreeSet::new();
    let mut found = None;
    for record in arr(&state["sessions"]) {
        for entry in aliases(&record)? {
            if !seen.insert(text(&entry, "requestId").to_owned()) {
                return Err(conflict(
                    "Preparation identities must belong to exactly one saved chat.",
                ));
            }
            if entry["requestId"] == id {
                found = Some((record.clone(), entry));
            }
        }
    }
    Ok(found)
}
fn receipt(record: &Value, entry: &Value) -> Value {
    json!({"requestId":entry["requestId"],"recordId":record["id"],"kind":entry["kind"],"status":"prepared","reused":entry["reused"]})
}
impl Request {
    pub fn lookup(&self, repo: &Repository, state: &Value) -> Result<Option<Value>> {
        let Some((mut record, entry)) = find(state, &self.id)? else {
            return Ok(None);
        };
        if entry["kind"] != self.kind || entry["hash"] != self.hash {
            return Err(conflict("This preparation identity already belongs to different instructions. Keep its original request or start a separate deliberate preparation."));
        }
        record_identity(repo, &record)?;
        record[RECEIPT] = receipt(&record, &entry);
        Ok(Some(record))
    }
}
pub(super) fn lookup(
    repo: &Repository,
    request: Option<&Request>,
    state: &Value,
) -> Result<Option<Value>> {
    match request {
        Some(request) => request.lookup(repo, state),
        None => Ok(None),
    }
}
pub(super) fn finish(
    repo: &Repository,
    request: Option<&Request>,
    state: &mut Value,
    record: Value,
    reused: bool,
) -> Result<Value> {
    let Some(request) = request else {
        return Ok(record);
    };
    record_identity(repo, &record)?;
    // The caller holds the canonical transaction; duplicate checks precede effects.
    if let Some(existing) = request.lookup(repo, state)? {
        return Ok(existing);
    }
    let rows = state["sessions"].as_array_mut().ok_or(Error::io())?;
    let saved = rows
        .iter_mut()
        .find(|row| row["id"] == record["id"])
        .ok_or(Error::new("Prepared chat is unavailable."))?;
    let mut entries = aliases(saved)?;
    if entries.len() >= MAX_ALIASES {
        return Err(conflict("This saved chat has reached its preparation identity limit. Existing preparations remain readable; open that saved chat instead of preparing it again."));
    }
    let entry =
        json!({"requestId":request.id,"kind":request.kind,"hash":request.hash,"reused":reused});
    entries.push(entry.clone());
    if saved.get("preparationRequests").is_none() {
        saved["preparationRequests"] = json!({"version":1});
    }
    saved["preparationRequests"]["entries"] = json!(entries);
    if serde_json::to_vec(&saved["preparationRequests"])?.len() > 64 * 1024 {
        return Err(conflict(
            "The saved preparation identities exceed their limit.",
        ));
    }
    let mut reply = saved.clone();
    reply[RECEIPT] = receipt(saved, &entry);
    Ok(reply)
}
fn input(kind: &str, args: &Value) -> Result<(String, Value)> {
    if !KINDS.contains(&kind) {
        return Err(Error::new("Unknown chat preparation kind."));
    }
    super::commands::validate_preparation_arguments(kind, args)?;
    if serde_json::to_vec(args)?.len() > 64 * 1024 {
        return Err(Error::new("Preparation input exceeds 64 KiB."));
    }
    let id = sid(text(args, "requestId"))?;
    let mut payload = args.clone();
    payload.as_object_mut().unwrap().remove("requestId");
    if payload
        .as_object()
        .unwrap()
        .values()
        .any(|value| value.as_str().is_some_and(|text| text.contains('\0')))
    {
        return Err(Error::new("Preparation text cannot contain NUL."));
    }
    if kind == "attach" {
        payload["sessionId"] = json!(sid(text(&payload, "sessionId"))?);
    }
    if kind == "recover" {
        payload["id"] = json!(sid(text(&payload, "id"))?);
    }
    Ok((id, payload))
}
pub fn prepare(repo: &Repository, kind: &str, args: &Value) -> Result<Value> {
    let (id, payload) = input(kind, args)?;
    projection(repo, &id)?;
    let hash = context_kit::sha256(&serde_json::to_vec(
        &json!({"version":1,"kind":kind,"intent":payload}),
    )?);
    let request = Request {
        id,
        kind: kind.to_owned(),
        hash,
    };
    {
        let state = repo.state.lock().map_err(|_| Error::io())?;
        if let Some(record) = request.lookup(repo, &state)? {
            record_identity(repo, &record)?;
            return Ok(record[RECEIPT].clone());
        }
    }
    let record = match kind {
        "quick" | "pr-review" => super::workflows::quick_with_request(
            repo,
            &payload,
            kind == "pr-review",
            Some(&request),
        )?,
        "review" => super::workflows::review_with_request(repo, &payload, Some(&request))?,
        "skill" => super::workflows::newsletter_with_request(repo, &payload, Some(&request))?,
        "recover" => super::workflows::recover_with_request(repo, &payload, Some(&request))?,
        "new" | "attach" => super::workflows::prepare_with_request(repo, &payload, Some(&request))?,
        "view" => {
            let mut payload = payload;
            payload["requestId"] = json!(request.id);
            super::view_context::prepare_with_request(repo, &payload, Some(&request))?
        }
        _ => unreachable!(),
    };
    record_identity(repo, &record)?;
    Ok(record[RECEIPT].clone())
}
pub fn read(repo: &Repository, args: &Value) -> Result<Value> {
    let object = args
        .as_object()
        .filter(|args| args.len() == 1 && args.contains_key("requestId"))
        .ok_or(Error::new("Preparation lookup accepts only requestId."))?;
    let id = sid(object["requestId"]
        .as_str()
        .ok_or(Error::new("Supply the preparation UUID."))?)?;
    projection(repo, &id)?;
    let state = repo.state.lock().map_err(|_| Error::io())?;
    match find(&state, &id)? {
        Some((record, entry)) => {
            record_identity(repo, &record)?;
            Ok(receipt(&record, &entry))
        }
        None => Ok(json!({"requestId":id,"status":"not-found"})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, std::sync::Arc<Repository>) {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        repo.transaction(|_| Ok(())).unwrap();
        (temp, repo)
    }
    fn new_args(repo: &Repository) -> Value {
        json!({"requestId":uuid(),"agent":"codex","title":"Prepared fixture","cwd":repo.root,"destination":"embedded","instruction":"Private preparation instruction","itemIds":[]})
    }
    #[test]
    fn preparation_new_identity_survives_lost_response_metadata_changes_and_restart() {
        let (temp, repo) = fixture();
        let args = new_args(&repo);
        let receipt = prepare(&repo, "new", &args).unwrap();
        assert_eq!(receipt.as_object().unwrap().len(), 5);
        assert_eq!(receipt["status"], "prepared");
        assert_eq!(receipt["reused"], false);
        assert!(repo.embedded.inventory().is_empty());
        let id = text(&receipt, "recordId");
        repo.transaction(|state| {
            let row = &mut state["sessions"][0];
            row["title"] = json!("Newer title");
            row["instruction"] = json!("Newer instruction");
            row["resources"] = json!([{"id":"preserved-resource"}]);
            row["privateNs"] = json!({"keep":true});
            Ok(())
        })
        .unwrap();
        let before = repo.state.lock().unwrap().clone();
        let disk = std::fs::read(&repo.data).unwrap();
        assert_eq!(prepare(&repo, "new", &args).unwrap(), receipt);
        assert_eq!(
            read(&repo, &json!({"requestId":args["requestId"]})).unwrap(),
            receipt
        );
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
        assert!(!receipt.to_string().contains("Private"));
        assert_eq!(before["sessions"][0]["id"], id);
        drop(repo);
        let reopened = Repository::open(temp.path().into(), None, true).unwrap();
        assert_eq!(prepare(&reopened, "new", &args).unwrap(), receipt);
        assert_eq!(arr(&reopened.state.lock().unwrap()["sessions"]).len(), 1);
        assert!(reopened.embedded.inventory().is_empty());
    }
    #[test]
    fn preparation_concurrent_same_request_commits_one_record_and_one_alias() {
        let (_temp, repo) = fixture();
        let args = new_args(&repo);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let mut threads = vec![];
        for _ in 0..2 {
            let repo = repo.clone();
            let args = args.clone();
            let barrier = barrier.clone();
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                prepare(&repo, "new", &args).unwrap()
            }));
        }
        barrier.wait();
        let first = threads.remove(0).join().unwrap();
        let second = threads.remove(0).join().unwrap();
        assert_eq!(first, second);
        let state = repo.state.lock().unwrap();
        assert_eq!(arr(&state["sessions"]).len(), 1);
        assert_eq!(aliases(&state["sessions"][0]).unwrap().len(), 1);
        assert!(repo.embedded.inventory().is_empty());
    }
    #[test]
    fn preparation_reused_requests_keep_all_original_aliases_and_latest_attached_metadata() {
        let (_temp, repo) = fixture();
        let mut args = new_args(&repo);
        args.as_object_mut().unwrap().remove("instruction");
        args["sessionId"] = json!(uuid());
        let first = prepare(&repo, "attach", &args).unwrap();
        let id = first["recordId"].clone();
        repo.transaction(|state| {
            let row = &mut state["sessions"][0];
            row["title"] = json!("Newer associated title");
            row["cwd"] = json!("/not-a-current-directory");
            row["privateNs"] = json!({"keep":true});
            row["items"] = json!([{"id":"latest-link"}]);
            row["preparationRequests"]["privateNs"] = json!({"keep":true});
            Ok(())
        })
        .unwrap();
        let mut second_args = args.clone();
        second_args["requestId"] = json!(uuid());
        second_args["title"] = json!("Stale title");
        let second = prepare(&repo, "attach", &second_args).unwrap();
        assert_eq!(second["recordId"], id);
        assert_eq!(second["reused"], true);
        assert_eq!(prepare(&repo, "attach", &args).unwrap(), first);
        assert_eq!(prepare(&repo, "attach", &second_args).unwrap(), second);
        let state = repo.state.lock().unwrap();
        let row = &state["sessions"][0];
        assert_eq!(row["title"], "Newer associated title");
        assert_eq!(row["cwd"], "/not-a-current-directory");
        assert_eq!(row["items"][0]["id"], "latest-link");
        assert_eq!(row["privateNs"]["keep"], true);
        assert_eq!(row["preparationRequests"]["privateNs"]["keep"], true);
        assert_eq!(aliases(row).unwrap().len(), 2);
    }
    #[test]
    fn preparation_changed_intent_and_malformed_state_fail_without_effects() {
        let (_temp, repo) = fixture();
        let args = new_args(&repo);
        prepare(&repo, "new", &args).unwrap();
        let before = repo.state.lock().unwrap().clone();
        let disk = std::fs::read(&repo.data).unwrap();
        let mut changed = args.clone();
        changed["title"] = json!("Different intent");
        assert_eq!(prepare(&repo, "new", &changed).unwrap_err().status, 409);
        let mut extra = args.clone();
        extra["id"] = json!(uuid());
        assert!(prepare(&repo, "new", &extra).is_err());
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
        repo.transaction(|state| {
            state["sessions"][0]["preparationRequests"]["version"] = json!(2);
            Ok(())
        })
        .unwrap();
        let disk = std::fs::read(&repo.data).unwrap();
        assert_eq!(
            prepare(&repo, "new", &new_args(&repo)).unwrap_err().status,
            409
        );
        assert!(read(&repo, &json!({"requestId":uuid()})).is_err());
        assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
        assert!(repo.embedded.inventory().is_empty());
    }
    #[test]
    fn preparation_full_alias_capacity_allows_duplicates_but_rejects_new_reuse_before_effects() {
        let (_temp, repo) = fixture();
        let mut args = new_args(&repo);
        args.as_object_mut().unwrap().remove("instruction");
        args["sessionId"] = json!(uuid());
        let first = prepare(&repo, "attach", &args).unwrap();
        repo.transaction(|state| {
            let entries = state["sessions"][0]["preparationRequests"]["entries"]
                .as_array_mut()
                .unwrap();
            for _ in 1..MAX_ALIASES {
                entries.push(
                    json!({"requestId":uuid(),"kind":"attach","hash":"0".repeat(64),"reused":true}),
                );
            }
            Ok(())
        })
        .unwrap();
        let disk = std::fs::read(&repo.data).unwrap();
        assert_eq!(prepare(&repo, "attach", &args).unwrap(), first);
        let mut next = args.clone();
        next["requestId"] = json!(uuid());
        assert_eq!(prepare(&repo, "attach", &next).unwrap_err().status, 409);
        assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
        assert_eq!(
            aliases(&repo.state.lock().unwrap()["sessions"][0])
                .unwrap()
                .len(),
            MAX_ALIASES
        );
    }
    #[test]
    fn preparation_failed_commit_has_no_alias_and_exact_retry_creates_once() {
        let (_temp, repo) = fixture();
        let args = new_args(&repo);
        let disk = std::fs::read(&repo.data).unwrap();
        let before = repo.state.lock().unwrap().clone();
        let temporary = repo.data.with_extension("tmp");
        std::fs::create_dir(&temporary).unwrap();
        assert!(prepare(&repo, "new", &args).is_err());
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
        assert_eq!(
            read(&repo, &json!({"requestId":args["requestId"]})).unwrap()["status"],
            "not-found"
        );
        std::fs::remove_dir(temporary).unwrap();
        let receipt = prepare(&repo, "new", &args).unwrap();
        assert_eq!(prepare(&repo, "new", &args).unwrap(), receipt);
        assert!(repo.embedded.inventory().is_empty());
    }
    #[test]
    fn preparation_read_is_state_only_and_duplicate_skill_bypasses_removed_configuration() {
        let (_temp, repo) = fixture();
        let preset = uuid();
        repo.transaction(|state|{state["skillPresets"]=json!([{"id":preset,"title":"Fixture","cwd":repo.root,"skill":"fixture.md","reports":"reports","command":"$fixture","agent":"codex"}]);Ok(())}).unwrap();
        std::fs::write(repo.root.join("fixture.md"), "Fixture skill").unwrap();
        let args = json!({"requestId":uuid(),"presetId":preset});
        let first = prepare(&repo, "skill", &args).unwrap();
        let config = repo.data.parent().unwrap().join("config/skills.json");
        std::fs::remove_file(config).unwrap();
        std::fs::remove_file(repo.root.join("fixture.md")).unwrap();
        let disk = std::fs::read(&repo.data).unwrap();
        assert_eq!(
            read(&repo, &json!({"requestId":args["requestId"]})).unwrap(),
            first
        );
        assert_eq!(prepare(&repo, "skill", &args).unwrap(), first);
        assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
        assert!(repo.embedded.inventory().is_empty());
    }
    #[test]
    fn preparation_view_failed_validation_and_persistence_do_not_consume_admission() {
        let (_temp, repo) = fixture();
        let args = json!({"requestId":uuid(),"agent":"codex","source":"github","question":"Which needs attention?","itemIds":["known"],"filters":{},"folderPath":repo.root});
        let cooldown = repo.cooldown.lock().unwrap().clone();
        assert!(prepare(&repo, "view", &args).is_err());
        assert_eq!(*repo.cooldown.lock().unwrap(), cooldown);
        repo.transaction(|state| {
            state["items"] = json!([{"id":"known","source":"github","title":"Known"}]);
            Ok(())
        })
        .unwrap();
        let disk = std::fs::read(&repo.data).unwrap();
        let temporary = repo.data.with_extension("tmp");
        std::fs::create_dir(&temporary).unwrap();
        assert!(prepare(&repo, "view", &args).is_err());
        assert_eq!(*repo.cooldown.lock().unwrap(), cooldown);
        assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
        std::fs::remove_dir(temporary).unwrap();
        let receipt = prepare(&repo, "view", &args).unwrap();
        assert_eq!(prepare(&repo, "view", &args).unwrap(), receipt);
        assert_eq!(arr(&repo.state.lock().unwrap()["sessions"]).len(), 1);
        assert!(repo.embedded.inventory().is_empty());
    }
    #[test]
    fn preparation_unverifiable_legacy_view_collision_is_rejected_without_writes_or_admission() {
        let (_temp, repo) = fixture();
        let request = uuid();
        let record_id = uuid();
        repo.transaction(|state| {
            state["sessions"] = json!([{"id":record_id,"viewRequestId":request,"title":"Legacy view","agent":"codex","cwd":repo.root,"instruction":"Inspect","launchNew":true,"items":[]}]);
            Ok(())
        }).unwrap();
        let before = repo.state.lock().unwrap().clone();
        let disk = std::fs::read(&repo.data).unwrap();
        let args = json!({"requestId":request,"agent":"codex","source":"github","question":"Inspect","itemIds":["not-required-for-existing-view"],"filters":{},"folderPath":repo.root});
        for question in ["Inspect", "Different question"] {
            let mut changed = args.clone();
            changed["question"] = json!(question);
            let error = prepare(&repo, "view", &changed).unwrap_err();
            assert_eq!(error.status, 409);
            assert!(error.message.contains("no verified intent hash"));
            assert_eq!(*repo.state.lock().unwrap(), before);
            assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
        }
        let legacy = super::super::view_context::prepare(&repo, &args).unwrap();
        assert_eq!(legacy["id"], record_id);
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
        assert!(!repo.data.parent().unwrap().join("views").exists());
        assert!(repo.cooldown.lock().unwrap().is_empty());
        assert!(repo.embedded.inventory().is_empty());
    }
    #[test]
    fn preparation_malformed_alias_record_identity_never_becomes_actionable() {
        let (_temp, repo) = fixture();
        let args = new_args(&repo);
        let first = prepare(&repo, "new", &args).unwrap();
        repo.transaction(|state| {
            state["sessions"][0]["id"] = json!("not-a-chat-uuid");
            Ok(())
        })
        .unwrap();
        let disk = std::fs::read(&repo.data).unwrap();
        assert!(read(&repo, &json!({"requestId":first["requestId"]})).is_err());
        assert!(prepare(&repo, "new", &args).is_err());
        assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
        assert!(repo.embedded.inventory().is_empty());
    }

    #[test]
    fn preparation_transaction_rechecks_identity_after_two_preflight_misses() {
        let (_temp, repo) = fixture();
        let args = new_args(&repo);
        let (id, payload) = input("new", &args).unwrap();
        let hash = context_kit::sha256(
            &serde_json::to_vec(&json!({"version":1,"kind":"new","intent":payload})).unwrap(),
        );
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let lock = repo.state.lock().unwrap();
        let mut threads = vec![];
        for _ in 0..2 {
            let repo = repo.clone();
            let payload = payload.clone();
            let request = Request {
                id: id.clone(),
                kind: "new".into(),
                hash: hash.clone(),
            };
            let barrier = barrier.clone();
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                super::super::workflows::prepare_with_request(&repo, &payload, Some(&request))
                    .unwrap()[RECEIPT]
                    .clone()
            }));
        }
        barrier.wait();
        drop(lock);
        let first = threads.remove(0).join().unwrap();
        assert_eq!(threads.remove(0).join().unwrap(), first);
        let state = repo.state.lock().unwrap();
        assert_eq!(arr(&state["sessions"]).len(), 1);
        assert_eq!(aliases(&state["sessions"][0]).unwrap().len(), 1);
    }
    #[test]
    fn preparation_multiple_reuse_aliases_survive_restart_without_retargeting() {
        let (temp, repo) = fixture();
        let mut args = new_args(&repo);
        args.as_object_mut().unwrap().remove("instruction");
        args["sessionId"] = json!(uuid());
        let mut receipts = vec![];
        for _ in 0..12 {
            args["requestId"] = json!(uuid());
            let receipt = prepare(&repo, "attach", &args).unwrap();
            receipts.push((args.clone(), receipt));
        }
        let target = receipts[0].1["recordId"].clone();
        assert!(receipts
            .iter()
            .all(|(_, receipt)| receipt["recordId"] == target));
        drop(repo);
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        let disk = std::fs::read(&repo.data).unwrap();
        for (args, receipt) in receipts {
            assert_eq!(prepare(&repo, "attach", &args).unwrap(), receipt);
        }
        assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
        assert_eq!(arr(&repo.state.lock().unwrap()["sessions"]).len(), 1);
    }
}
