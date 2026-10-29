use super::{arr, home, json, now, text, uuid, Error, PathBuf, Repository, Result, Value};
use std::{
    collections::BTreeMap,
    io::Read,
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
fn checked_action(repo: &Repository, payload: &Value) -> Result<Value> {
    let snapshot = repo.settings.preview(
        &json!({"revision":payload["revision"],"contentHash":payload["contentHash"],"patch":{}}),
        false,
    )?;
    arr(&snapshot["values"]["quickActions"])
        .into_iter()
        .find(|action| action["id"] == payload["actionId"])
        .ok_or(Error::new("Quick action unavailable."))
}
// Pipe reads can divide a UTF-8 scalar. Keep only its incomplete trailing bytes.
#[derive(Default)]
struct Utf8Output {
    pending: Vec<u8>,
}
impl Utf8Output {
    fn push_redacted(
        &mut self,
        bytes: &[u8],
        redactor: &mut super::providers::StreamRedactor,
    ) -> String {
        self.pending.extend_from_slice(bytes);
        let mut output = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(text) => {
                    output.push_str(&redactor.push(text));
                    self.pending.clear();
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    output.push_str(
                        &redactor.push(std::str::from_utf8(&self.pending[..valid]).unwrap()),
                    );
                    if let Some(length) = error.error_len() {
                        // Synthetic replacement cannot prove the preceding secret
                        // prefix safe. Close it before publishing the invalid byte.
                        output.push_str(&redactor.finish());
                        output.push_str(&redactor.push("\u{fffd}"));
                        self.pending.drain(..valid + length);
                    } else {
                        self.pending.drain(..valid);
                        break;
                    }
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
fn finish_stream(
    decoder: &mut Utf8Output,
    redactor: &mut super::providers::StreamRedactor,
) -> String {
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
    pub fn load(path: PathBuf) -> Result<Self> {
        let mut jobs = if path.is_file() {
            serde_json::from_slice::<BTreeMap<String, Value>>(
                &context_kit::read_bounded(&path, 1024 * 1024).map_err(Error::new)?,
            )?
        } else {
            BTreeMap::new()
        };
        for job in jobs.values_mut() {
            if job["state"] == "running" {
                job["state"] = json!("interrupted");
                job["message"] = json!("Service restarted. This run was not replayed.");
            }
        }
        let host = Self {
            jobs: Mutex::new(jobs),
            path: Some(path),
        };
        host.persist(&host.jobs.lock().unwrap())?;
        Ok(host)
    }
    fn persist(&self, jobs: &BTreeMap<String, Value>) -> Result<()> {
        if let Some(path) = &self.path {
            let temp = path.with_extension("tmp");
            super::http::private_write(&temp, &serde_json::to_vec(jobs)?)?;
            std::fs::rename(temp, path)?;
        }
        Ok(())
    }

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
            let _ = self.persist(&jobs);
        }
    }
    pub fn start(self: &Arc<Self>, repo: &Arc<Repository>, p: &Value) -> Result<Value> {
        if repo.demo {
            return Err(Error::new("Quick actions are disabled in the demo."));
        }
        let action = checked_action(repo, p)?;
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
            if (text(&action, "command").contains(&format!("${key}"))
                || text(&action, "command").contains(&format!("${{{key}}}")))
                && context.get(key).is_none_or(std::string::String::is_empty)
            {
                return Err(Error::new(format!(
                    "Choose a chat, worktree or work item to supply ${key}."
                )));
            }
        }
        let cwd = context
            .get("WORKTREE_PATH")
            .or(context.get("CHAT_CWD"))
            .filter(|s| !s.is_empty())
            .map_or_else(home, PathBuf::from);
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
                        if let Some(job) = host.jobs.lock().unwrap().get_mut(&id) {
                            job["exitCode"] = json!(status.code());
                        }
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
                    });
                }
                Err(e) => {
                    job["state"] = json!("failed");
                    job["output"] = json!(e.message);
                }
            }
            let _ = host.persist(&jobs);
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
                .next_back()
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
mod tests;
