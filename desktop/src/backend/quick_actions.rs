use super::*;
use std::{
    collections::BTreeMap,
    io::Read,
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
fn checked_action(repo: &Repository, payload: &Value) -> Result<Value> {
    let snapshot=repo.settings.preview(&json!({"revision":payload["revision"],"contentHash":payload["contentHash"],"patch":{}}),false)?;
    arr(&snapshot["values"]["quickActions"]).into_iter().find(|action|action["id"]==payload["actionId"])
        .ok_or(Error::new("Quick action unavailable."))
}
// Pipe reads can divide a UTF-8 scalar. Keep only its incomplete trailing bytes.
#[derive(Default)]
struct Utf8Output { pending: Vec<u8> }
impl Utf8Output {
    fn push_redacted(&mut self, bytes: &[u8], redactor: &mut super::providers::StreamRedactor) -> String {
        self.pending.extend_from_slice(bytes);
        let mut output = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(text) => { output.push_str(&redactor.push(text)); self.pending.clear(); break; }
                Err(error) => {
                    let valid = error.valid_up_to();
                    output.push_str(&redactor.push(std::str::from_utf8(&self.pending[..valid]).unwrap()));
                    if let Some(length) = error.error_len() {
                        // Synthetic replacement cannot prove the preceding secret
                        // prefix safe. Close it before publishing the invalid byte.
                        output.push_str(&redactor.finish());
                        output.push_str(&redactor.push("\u{fffd}"));
                        self.pending.drain(..valid + length);
                    } else { self.pending.drain(..valid); break; }
                }
            }
        }
        output
    }
    fn finish(&mut self) -> String {
        let tail = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        tail
    }
}
// An incomplete EOF scalar is not evidence that a withheld secret prefix was
// benign. Finalize that prefix first, then preserve the invalid-byte marker.
fn finish_stream(decoder: &mut Utf8Output, redactor: &mut super::providers::StreamRedactor) -> String {
    let mut output = redactor.finish();
    output.push_str(&redactor.push(&decoder.finish()));
    output.push_str(&redactor.finish());
    output
}
#[derive(Default)]
pub struct Host {
    jobs: Mutex<BTreeMap<String, Value>>,
    path: Option<PathBuf>,
}
impl Host {
    pub fn load(path:PathBuf)->Result<Self>{
        let mut jobs=if path.is_file(){serde_json::from_slice::<BTreeMap<String,Value>>(&context_kit::read_bounded(&path,1024*1024).map_err(Error::new)?)?}else{BTreeMap::new()};
        for job in jobs.values_mut(){if job["state"]=="running"{job["state"]=json!("interrupted");job["message"]=json!("Service restarted. This run was not replayed.");}}
        let host=Self{jobs:Mutex::new(jobs),path:Some(path)};host.persist(&host.jobs.lock().unwrap())?;Ok(host)
    }
    fn persist(&self,jobs:&BTreeMap<String,Value>)->Result<()>{if let Some(path)=&self.path{let temp=path.with_extension("tmp");super::http::private_write(&temp,&serde_json::to_vec(jobs)?)?;std::fs::rename(temp,path)?;}Ok(())}

    pub fn view(&self) -> Value {
        json!({"jobs":self.jobs.lock().unwrap().values().cloned().collect::<Vec<_>>()})
    }
    fn output(&self, id: &str, chunk: &str) {
        let mut jobs = self.jobs.lock().unwrap();
        if let Some(job) = jobs.get_mut(id) {
            let mut out = text(job, "output").to_owned();
            out.push_str(chunk);
            if out.len() > 16000 {
                let mut cut = out.len() - 16000;
                while !out.is_char_boundary(cut) {
                    cut += 1;
                }
                out = out[cut..].to_owned();
            }
            job["output"] = json!(out);
            let _=self.persist(&jobs);
        }
    }
    pub fn start(self: &Arc<Self>, repo: &Arc<Repository>, p: &Value) -> Result<Value> {
        if repo.demo {
            return Err(Error::new("Quick actions are disabled in the demo."));
        }
        let action = checked_action(repo,p)?;
        let context = context(repo, p)?;
        for key in [
            "CHAT_ID",
            "CHAT_TITLE",
            "CHAT_CWD",
            "CHAT_AGENT",
            "CHAT_SESSION_ID",
            "WORKTREE_PATH",
            "PR_URL",
            "PR_NUMBER",
            "TICKET_URL",
            "TICKET_ID",
        ] {
            if text(&action, "command").contains(&format!("${key}"))
                || text(&action, "command").contains(&format!("${{{key}}}"))
            {
                if context.get(key).is_none_or(|s| s.is_empty()) {
                    return Err(Error::new(format!(
                        "Choose a chat, worktree or work item to supply ${key}."
                    )));
                }
            }
        }
        let cwd = context
            .get("WORKTREE_PATH")
            .or(context.get("CHAT_CWD"))
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(home);
        let id = uuid();
        let job = json!({"command":action["command"],"cwd":cwd,"id":id,"actionId":action["id"],"title":action["title"],"state":"running","startedAt":now(),"output":""});
        {
            let mut jobs = self.jobs.lock().unwrap();
            if jobs.values().filter(|j| j["state"] == "running").count() >= 4
                || jobs
                    .values()
                    .any(|j| j["state"] == "running" && j["actionId"] == action["id"])
            {
                return Err(Error::new(
                    "This action is already running, or four actions are active.",
                ));
            }
            let mut next = jobs.clone();
            if next.len() >= 30 {
                let oldest = next
                    .iter()
                    .filter(|(_, j)| j["state"] != "running")
                    .min_by(|(_, a), (_, b)| {
                        a["startedAt"]
                            .as_f64()
                            .partial_cmp(&b["startedAt"].as_f64())
                            .unwrap()
                    })
                    .map(|(k, _)| k.clone());
                if let Some(k) = oldest {
                    next.remove(&k);
                }
            }
            next.insert(id.clone(), job);
            self.persist(&next)?;
            *jobs = next;
        }
        let reply_id = id.clone();
        let host = self.clone();
        let repo = repo.clone();
        std::thread::spawn(move || {
            let mut command = Command::new("/bin/zsh");
            command
                .args(["-lc", text(&action, "command")])
                .current_dir(cwd)
                .envs(context)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let result = (|| -> Result<(bool, bool)> {
                let mut child = command.spawn()?;
                let stdout = child.stdout.take().unwrap();
                let stderr = child.stderr.take().unwrap();
                let readers = [
                    Box::new(stdout) as Box<dyn Read + Send>,
                    Box::new(stderr) as Box<dyn Read + Send>,
                ]
                .into_iter()
                .map(|mut stream| {
                    let host = host.clone();
                    let id = id.clone();
                    let mut redactor = super::commands::stream_redactor(&repo);
                    std::thread::spawn(move || {
                        let mut decoder = Utf8Output::default();
                        let mut bytes = [0; 2048];
                        while let Ok(n) = stream.read(&mut bytes) {
                            if n == 0 {
                                break;
                            }
                            host.output(&id, &decoder.push_redacted(&bytes[..n], &mut redactor));
                        }
                        host.output(&id, &finish_stream(&mut decoder, &mut redactor));
                    })
                })
                .collect::<Vec<_>>();
                let start = Instant::now();
                let mut timed_out = false;
                let success = loop {
                    if let Some(status) = child.try_wait()? {
                        if let Some(job)=host.jobs.lock().unwrap().get_mut(&id){job["exitCode"]=json!(status.code());}
                        break status.success();
                    }
                    if start.elapsed() > Duration::from_secs(600) {
                        timed_out = true;
                        unsafe {
                            libc::kill(-(child.id() as i32), libc::SIGKILL);
                        }
                        let _ = child.wait();
                        break false;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                };
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                for reader in readers {
                    let _ = reader.join();
                }
                Ok((success, timed_out))
            })();
            let mut jobs = host.jobs.lock().unwrap();
            let job = jobs.get_mut(&id).unwrap();
            job["finishedAt"] = json!(now());
            match result {
                Ok((success, timed_out)) => {
                    job["state"] = json!(if timed_out {
                        "timed_out"
                    } else if success {
                        "succeeded"
                    } else {
                        "failed"
                    })
                }
                Err(e) => {
                    job["state"] = json!("failed");
                    job["output"] = json!(e.message);
                }
            }
            let _=host.persist(&jobs);
        });
        Ok(json!({"id":reply_id}))
    }
}
fn context(repo: &Repository, p: &Value) -> Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    if text(p, "recordId").is_empty() {
        return Ok(result);
    }
    let state = repo.state.lock().map_err(|_| Error::io())?;
    let record = arr(&state["sessions"])
        .into_iter()
        .find(|r| r["id"] == p["recordId"])
        .ok_or(Error::new("Selected chat unavailable."))?;
    for (env, key) in [
        ("CHAT_ID", "id"),
        ("CHAT_TITLE", "title"),
        ("CHAT_CWD", "cwd"),
        ("CHAT_AGENT", "agent"),
        ("CHAT_SESSION_ID", "sessionId"),
    ] {
        result.insert(env.into(), text(&record, key).into());
    }
    let cwd = if text(p, "cwd").is_empty() {
        text(&record, "cwd")
    } else {
        text(p, "cwd")
    };
    if cwd != text(&record, "cwd") && !arr(&record["worktrees"]).iter().any(|t| t["path"] == cwd) {
        return Err(Error::new("Choose a worktree linked to this chat."));
    }
    result.insert("WORKTREE_PATH".into(), cwd.into());
    for (field, source, prefix) in [("prId", "github", "PR"), ("ticketId", "linear", "TICKET")] {
        if text(p, field).is_empty() {
            continue;
        }
        let item = arr(&record["items"])
            .into_iter()
            .find(|i| i["id"] == p[field] && i["source"] == source)
            .ok_or(Error::new("Choose a work item linked to this chat."))?;
        result.insert(format!("{prefix}_URL"), text(&item, "url").into());
        if prefix == "PR" {
            let number = text(&item, "url")
                .split('/')
                .last()
                .unwrap_or("")
                .to_string();
            if !text(&item, "url").contains("/pull/") || !number.bytes().all(|b| b.is_ascii_digit())
            {
                return Err(Error::new("Choose a pull request."));
            }
            result.insert("PR_NUMBER".into(), number);
        } else {
            result.insert("TICKET_ID".into(), text(&item, "reference").into());
        }
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn output_fixture(secret: &str) -> (tempfile::TempDir, Arc<Repository>) {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(".env"), format!("OUTPUT_TEST_SECRET={secret}\n")).unwrap();
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        (temp, repo)
    }
    fn output_job(repo: &Repository, id: &str) {
        repo.quick_actions.jobs.lock().unwrap().insert(id.into(), json!({"id":id,"state":"running","startedAt":0,"output":""}));
    }
    fn output_text(repo: &Repository, id: &str) -> String {
        text(&repo.quick_actions.jobs.lock().unwrap()[id], "output").to_owned()
    }
    #[test]
    fn stream_output_keeps_unicode_and_redacts_every_secret_byte_boundary() {
        let secret = "benign-stream-é秘密-0123456789";
        let (_temp, repo) = output_fixture(secret);
        for split in 0..=secret.len() {
            let id = format!("split-{split}");
            output_job(&repo, &id);
            let mut decoder = Utf8Output::default();
            let mut redactor = super::super::commands::stream_redactor(&repo);
            repo.quick_actions.output(&id, &decoder.push_redacted(b"Live safe output: ", &mut redactor));
            assert_eq!(output_text(&repo, &id), "Live safe output: ");
            repo.quick_actions.output(&id, &decoder.push_redacted(&secret.as_bytes()[..split], &mut redactor));
            assert!(!output_text(&repo, &id).contains("benign-stream"));
            repo.quick_actions.output(&id, &decoder.push_redacted(&secret.as_bytes()[split..], &mut redactor));
            for byte in " café 🔎 end".as_bytes() {
                repo.quick_actions.output(&id, &decoder.push_redacted(&[*byte], &mut redactor));
            }
            repo.quick_actions.output(&id, &finish_stream(&mut decoder, &mut redactor));
            assert_eq!(output_text(&repo, &id), "Live safe output: [redacted] café 🔎 end");
            let public = super::super::agent_access::quick_action_outputs(&repo, &json!({"id":id}));
            assert_eq!(public["jobs"][0]["output"], "Live safe output: [redacted] café 🔎 end");
        }
    }
    #[test]
    fn stream_output_truncated_unicode_secret_eof_never_releases_withheld_prefix() {
        let secret = "fixture-é-secret";
        let (_temp, repo) = output_fixture(secret);
        for end in 0..=secret.len() {
            let id = format!("eof-{end}"); output_job(&repo, &id);
            let bytes = &secret.as_bytes()[..end];
            let mut decoder = Utf8Output::default();
            let mut redactor = super::super::commands::stream_redactor(&repo);
            for byte in bytes {
                repo.quick_actions.output(&id, &decoder.push_redacted(&[*byte], &mut redactor));
            }
            repo.quick_actions.output(&id, &finish_stream(&mut decoder, &mut redactor));
            let expected = if end == 0 { "" } else if std::str::from_utf8(bytes).is_err() { "[redacted]�" } else { "[redacted]" };
            assert_eq!(output_text(&repo, &id), expected, "truncated byte {end}");
            let public = super::super::agent_access::quick_action_outputs(&repo, &json!({"id":id}));
            assert_eq!(public["jobs"][0]["output"], expected);
        }
        for (bytes, expected) in [(b"plain \xc3".as_slice(), "plain �"), (b"plain \xff end".as_slice(), "plain � end")] {
            let mut decoder = Utf8Output::default();
            let mut redactor = super::super::commands::stream_redactor(&repo);
            let mut output = decoder.push_redacted(bytes, &mut redactor);
            output.push_str(&finish_stream(&mut decoder, &mut redactor));
            assert_eq!(output, expected);
        }
    }
    #[test]
    fn stream_output_invalid_unicode_boundaries_never_release_withheld_prefixes() {
        let secret = "fixture-é秘密-secret";
        let (_temp, repo) = output_fixture(secret);
        for end in 1..secret.len() {
            let prefix = &secret.as_bytes()[..end];
            if std::str::from_utf8(prefix).is_ok() { continue; }
            for suffix in [b"X".as_slice(), b"\n".as_slice()] {
                let id = format!("invalid-{end}-{}", suffix[0]); output_job(&repo, &id);
                let mut decoder = Utf8Output::default();
                let mut redactor = super::super::commands::stream_redactor(&repo);
                repo.quick_actions.output(&id, &decoder.push_redacted(prefix, &mut redactor));
                assert_eq!(output_text(&repo, &id), "");
                repo.quick_actions.output(&id, &decoder.push_redacted(suffix, &mut redactor));
                let expected = format!("[redacted]�{}", std::str::from_utf8(suffix).unwrap());
                // Output must remain live before EOF as well as safe afterward.
                assert_eq!(output_text(&repo, &id), expected);
                repo.quick_actions.output(&id, &finish_stream(&mut decoder, &mut redactor));
                assert_eq!(output_text(&repo, &id), expected);
                let public = super::super::agent_access::quick_action_outputs(&repo, &json!({"id":id}));
                assert_eq!(public["jobs"][0]["output"], expected);
            }
        }
        let mut decoder = Utf8Output::default();
        let mut redactor = super::super::commands::stream_redactor(&repo);
        assert_eq!(decoder.push_redacted(b"plain \xc3", &mut redactor), "plain ");
        assert_eq!(decoder.push_redacted(b"X\n", &mut redactor), "�X\n");
        assert_eq!(finish_stream(&mut decoder, &mut redactor), "");
    }
    #[test]
    fn stream_output_redacts_long_known_values_before_owner_tail_cut() {
        let secret = format!("benign-long-secret-{}-end", "é".repeat(10000));
        let (_temp, repo) = output_fixture(&secret);
        let id = "tail"; output_job(&repo, id);
        let raw = format!("safe prefix {secret}{}", "z".repeat(15990));
        let mut decoder = Utf8Output::default();
        let mut redactor = super::super::commands::stream_redactor(&repo);
        for bytes in raw.as_bytes().chunks(2048) {
            repo.quick_actions.output(id, &decoder.push_redacted(bytes, &mut redactor));
        }
        repo.quick_actions.output(id, &finish_stream(&mut decoder, &mut redactor));
        assert_eq!(output_text(&repo, id), format!("[redacted]{}", "z".repeat(15990)));
        let public = super::super::agent_access::quick_action_outputs(&repo, &json!({"id":id}));
        assert_eq!(public["jobs"][0]["output"], "z".repeat(2000));
    }
    #[test]
    fn stream_output_withholds_ambiguous_prefixes_and_bounds_pending_sensitive_text() {
        let (_temp, repo) = output_fixture("fixture-short");
        let mut redactor = repo.providers.stream_redactor().with_values(["fixture-short-longer".into()]);
        assert_eq!(redactor.push("visible fixture-short"), "visible ");
        assert_eq!(redactor.push("-long"), "");
        assert_eq!(redactor.push("er done"), "[redacted] done");
        assert_eq!(redactor.finish(), "");
        let mut partial = repo.providers.stream_redactor();
        assert_eq!(partial.push("safe fixture-sh"), "safe ");
        assert_eq!(partial.finish(), "[redacted]");
        let huge = format!("{}end", "q".repeat(65536));
        let mut bounded = repo.providers.stream_redactor().with_values([huge]);
        assert_eq!(bounded.push(&"q".repeat(65535)), "");
        let notice = bounded.push("q");
        assert_eq!(notice, "[Output omitted: sensitive value exceeds stream safety limit.]");
        assert_eq!(bounded.push("end subsequent text"), "");
        assert_eq!(bounded.finish(), "");
    }
    #[test]
    fn reviewed_command_identity_is_checked_before_lookup_without_execution() {
        let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().into(),None,true).unwrap();
        repo.settings.apply(&json!({"revision":0,"contentHash":repo.settings.view()["contentHash"],"patch":{"quickActions":[{"id":"fixture","title":"Fixture","command":"printf reviewed"}]}}),false).unwrap();
        let baseline=repo.settings.view();let request=json!({"revision":baseline["revision"],"contentHash":baseline["contentHash"],"actionId":"fixture"});
        assert_eq!(checked_action(&repo,&request).unwrap()["command"],"printf reviewed");
        let path=PathBuf::from(text(&baseline["files"],"preferences"));let mut external:Value=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();external["values"]["quickActions"][0]["command"]=json!("printf changed");
        std::fs::write(&path,serde_json::to_vec(&external).unwrap()).unwrap();let bytes=std::fs::read(&path).unwrap();
        assert_eq!(checked_action(&repo,&request).unwrap_err().status,409);
        let fresh=repo.settings.view();assert_eq!(checked_action(&repo,&json!({"revision":fresh["revision"],"contentHash":baseline["contentHash"],"actionId":"fixture"})).unwrap_err().status,409);
        assert_eq!(checked_action(&repo,&json!({"revision":fresh["revision"],"contentHash":fresh["contentHash"],"actionId":"fixture"})).unwrap()["command"],"printf changed");
        assert_eq!(std::fs::read(&path).unwrap(),bytes);assert!(arr(&repo.quick_actions.view()["jobs"]).is_empty());
    }

    #[test]
    fn failed_start_persistence_preserves_jobs_and_allows_retry() {
        let root = tempfile::tempdir().unwrap();
        let repo = Repository::open(root.path().join("profile"), None, false).unwrap();
        repo.settings.apply(&json!({"revision":0,"contentHash":repo.settings.view()["contentHash"],"patch":{"quickActions":[{"id":"fixture","title":"Fixture","command":"printf retry"}]}}), false).unwrap();
        let path = root.path().join("runs.json");
        let host = Arc::new(Host::load(path.clone()).unwrap());
        {
            let mut jobs = host.jobs.lock().unwrap();
            for index in 0..30 {
                jobs.insert(format!("old-{index}"), json!({"id":format!("old-{index}"),"state":"succeeded","startedAt":index,"output":"saved"}));
            }
            host.persist(&jobs).unwrap();
        }
        let baseline = host.view();
        let bytes = std::fs::read(&path).unwrap();
        let blocked_temp = path.with_extension("tmp");
        std::fs::create_dir(&blocked_temp).unwrap();
        let request = json!({"revision":1,"contentHash":repo.settings.view()["contentHash"],"actionId":"fixture"});
        assert!(host.start(&repo, &request).is_err());
        assert_eq!(host.view(), baseline);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        std::fs::remove_dir(blocked_temp).unwrap();
        let run = host.start(&repo, &request).unwrap();
        let start = Instant::now();
        loop {
            let job = arr(&host.view()["jobs"]).into_iter().find(|job| job["id"] == run["id"]).unwrap();
            if job["state"] != "running" {
                assert_eq!(job["state"], "succeeded");
                assert_eq!(job["output"], "retry");
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(arr(&host.view()["jobs"]).len(), 30);
    }

    #[test]
    fn persisted_output_survives_service_restart_without_replaying_commands(){
        let root=tempfile::tempdir().unwrap();let path=root.path().join("runs.json");let host=Host::load(path.clone()).unwrap();{let mut jobs=host.jobs.lock().unwrap();jobs.insert("run".into(),json!({"state":"running","output":"diagnostic output"}));host.persist(&jobs).unwrap();}drop(host);let restored=Host::load(path).unwrap();let value=restored.view();assert_eq!(value["jobs"][0]["output"],"diagnostic output");assert_eq!(value["jobs"][0]["state"],"interrupted");
    }
    #[test]
    fn quick_action_context_is_literal_and_rejects_unlinked_items() {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::open(temp.path().to_path_buf(), None, false).unwrap();
        let id = uuid();
        let title = "$(printf injected) ; quoted ' \" title";
        repo.transaction(|s| {
            s["sessions"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":id,"agent":"claude","title":title,"cwd":temp.path(),"items":[]}));
            Ok(())
        })
        .unwrap();
        let context = context(&repo, &json!({"recordId":id})).unwrap();
        assert_eq!(context["CHAT_TITLE"], title);
        assert!(super::context(&repo, &json!({"recordId":id,"prId":"unlinked"})).is_err());
        repo.settings.apply(&json!({"revision":0,"contentHash":repo.settings.view()["contentHash"],"patch":{"quickActions":[{"id":"literal","title":"Literal","command":"printf '%s' \"$CHAT_TITLE\""}]}}),false).unwrap();
        let run = repo
            .quick_actions
            .start(
                &repo,
                &json!({"revision":1,"contentHash":repo.settings.view()["contentHash"],"actionId":"literal","recordId":id}),
            )
            .unwrap();
        let start = Instant::now();
        loop {
            let view = repo.quick_actions.view();
            let job = arr(&view["jobs"])
                .into_iter()
                .find(|j| j["id"] == run["id"])
                .unwrap();
            if job["state"] != "running" {
                assert_eq!(job["state"], "succeeded");
                assert_eq!(job["output"], title);
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}
