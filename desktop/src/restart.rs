//! Desktop-controlled service/terminal restarts, including bootstrap from older services.
//! A detached helper survives UI closure. Only verified processes for this profile are signalled.
use crate::{
    backend::{self, arr, text},
    service::{DashboardService, LocalHttpService},
};
use fs2::FileExt;
use serde_json::{json, Value};
use std::{
    fs::{File, OpenOptions},
    os::unix::{
        fs::{OpenOptionsExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, String>;
fn io<T>(r: std::io::Result<T>) -> Result<T> {
    r.map_err(|e| format!("Restart operation failed: {e}"))
}
pub fn root() -> PathBuf {
    std::env::var_os("TRUFFLE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| backend::home().join("code/mydash"))
}
fn data(root: &Path) -> PathBuf {
    root.join("data/state.json")
}
fn directory(root: &Path) -> Result<PathBuf> {
    let path = root.join("data/restarts");
    io(std::fs::create_dir_all(&path))?;
    io(std::fs::set_permissions(
        &path,
        std::fs::Permissions::from_mode(0o700),
    ))?;
    Ok(path)
}
fn read(path: &Path) -> Result<Value> {
    let bytes = context_kit::read_bounded(path, 1024 * 1024).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|_| "Invalid restart metadata.".into())
}
fn read_state(root: &Path) -> Result<Value> {
    let bytes =
        context_kit::read_bounded(&data(root), 64 * 1024 * 1024).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|_| "Invalid dashboard state.".into())
}
fn write(path: &Path, value: &Value) -> Result<()> {
    let temp = path.with_extension(format!("{}.tmp", backend::uuid()));
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    backend::http::private_write(&temp, &bytes).map_err(|e| e.message)?;
    io(std::fs::rename(temp, path))
}
fn lock(path: &Path) -> Result<File> {
    let file = io(OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path))?;
    file.try_lock_exclusive()
        .map_err(|_| "Another restart is already running.".to_owned())?;
    Ok(file)
}
fn api(url: &str, path: &str, payload: Option<Value>) -> Result<Value> {
    let service = LocalHttpService::new(url)?;
    let reply = service.request(path, payload)?;
    if !(200..300).contains(&reply.status) {
        return Err(text(&reply.data, "error").to_owned());
    }
    Ok(reply.data)
}
fn port(url: &str) -> Result<u16> {
    LocalHttpService::new(url)?;
    reqwest::Url::parse(url)
        .map_err(|_| "Invalid local service URL.")?
        .port_or_known_default()
        .ok_or("Missing service port.".into())
}
fn runtime(root: &Path, url: &str) -> Result<Value> {
    let v = read(&root.join("data/runtime.json"))?;
    if text(&v, "url").trim_end_matches('/') != url.trim_end_matches('/') {
        return Err(
            "This app is connected to another profile. Restart it from its own installation."
                .into(),
        );
    }
    if let Some(path) = v["data"].as_str() {
        if Path::new(path).canonicalize().ok() != data(root).canonicalize().ok() {
            return Err(
                "Restart controls require this installation's default data/state.json profile."
                    .into(),
            );
        }
    }
    Ok(v)
}
fn output(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|_| format!("Could not inspect the local {program} process."))?;
    if !out.status.success() {
        return Ok(String::new());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
fn matches_process(pid: u32, mode: &str, root: &Path) -> bool {
    if pid < 2 {
        return false;
    }
    let id = pid.to_string();
    let Ok(uid) = output("/bin/ps", &["-p", &id, "-o", "uid="]) else {
        return false;
    };
    if uid.parse::<u32>().ok() != Some(unsafe { libc::geteuid() }) {
        return false;
    }
    let Ok(executable) = output("/bin/ps", &["-p", &id, "-o", "comm="]) else {
        return false;
    };
    if !executable.ends_with("/truffle-desktop") && !executable.ends_with("/mydash-desktop") {
        return false;
    }
    let Ok(command) = output("/bin/ps", &["-p", &id, "-o", "command="]) else {
        return false;
    };
    command.split_whitespace().any(|x| x == mode)
        && argument_matches(&command, "--home", &root.to_string_lossy())
}
fn argument_matches(command: &str, key: &str, value: &str) -> bool {
    let wanted = format!("{key} {value}");
    command.match_indices(&wanted).any(|(start, _)| {
        (start == 0 || command.as_bytes()[start - 1] == b' ')
            && (start + wanted.len() == command.len()
                || command.as_bytes()[start + wanted.len()] == b' ')
    })
}
fn service_pid(root: &Path, url: &str) -> Result<Option<u32>> {
    let selector = format!("-iTCP:{}", port(url)?);
    let ids = output("/usr/sbin/lsof", &["-t", &selector, "-sTCP:LISTEN"])?;
    let ids = ids
        .split_whitespace()
        .filter_map(|x| x.parse::<u32>().ok())
        .collect::<std::collections::BTreeSet<_>>();
    if ids.is_empty() {
        return Ok(None);
    }
    if ids.len() != 1 {
        return Err("More than one process owns the service port. No process was stopped.".into());
    }
    let pid = *ids.first().unwrap();
    if !matches_process(pid, "--serve", root) {
        return Err(
            "The service port belongs to a different process or profile. No process was stopped."
                .into(),
        );
    }
    Ok(Some(pid))
}
fn daemon(root: &Path) -> Result<Option<Value>> {
    let dir = backend::terminal_daemon::socket_directory(&data(root)).map_err(|e| e.message)?;
    if !dir.join("terminal.sock").exists() {
        return Ok(None);
    }
    match backend::terminal_daemon::call(&dir, json!({"op":"health"})) {
        Ok(health) => {
            if health["root"] != json!(root)
                || !health["pid"]
                    .as_u64()
                    .is_some_and(|p| matches_process(p as u32, "--terminal-daemon", root))
            {
                return Err("Could not verify the terminal daemon for this profile.".into());
            }
            Ok(Some(health))
        }
        Err(e) => {
            let file = io(OpenOptions::new()
                .read(true)
                .write(true)
                .open(dir.join("daemon.lock")))?;
            if file.try_lock_exclusive().is_ok() {
                Ok(None)
            } else {
                Err(e.message)
            }
        }
    }
}
fn daemon_request(root: &Path, p: Value) -> Result<Value> {
    let dir = backend::terminal_daemon::socket_directory(&data(root)).map_err(|e| e.message)?;
    backend::terminal_daemon::call(&dir, p).map_err(|e| e.message)
}
fn inspect(root: &Path, url: &str) -> Result<Value> {
    runtime(root, url)?;
    let pid = service_pid(root, url)?;
    let host = daemon(root)?;
    let inventory = if pid.is_some() {
        api(url, "/api/agents", None)?
    } else {
        json!({"candidates":if host.is_some(){daemon_request(root,json!({"op":"inventory"}))?}else{json!([])}})
    };
    if !inventory["warning"].is_null() {
        return Err(
            "Live terminal status is unavailable. Restore the connection before restarting.".into(),
        );
    }
    let live = arr(&inventory["candidates"])
        .into_iter()
        .filter(|c| c["live"] == true)
        .collect::<Vec<_>>();
    let daemon_live = if host.is_some() {
        arr(&daemon_request(root, json!({"op":"inventory"}))?)
    } else {
        vec![]
    };
    let preserved = host.is_some()
        && live.iter().all(|c| {
            daemon_live.iter().any(|d| {
                d["recordId"] == c["recordId"]
                    && d["generation"] == c["generation"]
                    && d["live"] == true
            })
        });
    let state = read_state(root)?;
    let sessions = arr(&state["sessions"]);
    let mut records = vec![];
    let mut problems = vec![];
    for candidate in &live {
        if let Some(record) = sessions.iter().find(|r| r["id"] == candidate["recordId"]) {
            if backend::sid(text(record, "sessionId")).is_err()
                || !backend::expand(text(record, "cwd")).is_dir()
            {
                problems.push(json!(format!("{} needs a saved chat UUID and an existing working directory before it can resume.",text(record,"title"))));
            }
            records.push(json!({"id":record["id"],"sessionId":record["sessionId"],"agent":record["agent"],"title":record["title"],"cwd":record["cwd"],"generation":candidate["generation"]}));
        } else {
            problems.push(json!(
                "A live terminal is not attached to a saved conversation."
            ));
        }
    }
    records.sort_by(|a, b| text(a, "id").cmp(text(b, "id")));
    let snapshot = context_kit::sha256(
        &serde_json::to_vec(&json!([
            pid,
            host.as_ref().map(|h| h["pid"].clone()),
            records
        ]))
        .unwrap(),
    );
    Ok(
        json!({"servicePid":pid,"daemonPid":host.as_ref().map(|h|h["pid"].clone()),"preserveTerminals":preserved,"liveCount":live.len(),"sessions":records,"problems":problems,"snapshot":snapshot,"serviceBuiltAt":runtime(root,url)?["builtAt"],"appBuiltAt":env!("TRUFFLE_BUILD_TIME")}),
    )
}
pub fn preview(root: &Path, url: &str) -> Result<Value> {
    let root = io(root.canonicalize())?;
    let mut value = inspect(&root, url)?;
    value["job"] = status(&root)?;
    Ok(value)
}
pub fn status(root: &Path) -> Result<Value> {
    let path = directory(root)?.join("current.json");
    if !path.exists() {
        return Ok(json!({"status":"idle"}));
    }
    let job = read(&path)?;
    if job["status"] == "working"
        && backend::now() - job["startedAt"].as_f64().unwrap_or(0.0) > 120.0
        && backend::sid(text(&job, "id")).is_ok()
    {
        // An interrupted helper must not leave a permanently spinning control.
        // A live worker retains this lock throughout the complete operation.
        if let Ok(_guard) = lock(&directory(root)?.join("worker.lock")) {
            return patch(
                root,
                text(&job, "id"),
                json!({"status":"failed","phase":"finished","finishedAt":backend::now(),"message":"The restart helper stopped before completion. Check the service and resume any missing chats from History."}),
            );
        }
    }
    Ok(job)
}
fn latest_binary() -> Result<PathBuf> {
    let installed = backend::home().join("Applications/truffle.app/Contents/MacOS/truffle-desktop");
    if installed.is_file() {
        Ok(installed)
    } else {
        io(std::env::current_exe())
    }
}
fn detached(command: &mut Command) -> Result<std::process::Child> {
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    io(command.spawn())
}
pub fn start(root: &Path, url: &str, mode: &str, confirmed: bool, snapshot: &str) -> Result<Value> {
    if !["service", "terminals"].contains(&mode) {
        return Err("Choose a service or terminal restart.".into());
    }
    let root = io(root.canonicalize())?;
    let dir = directory(&root)?;
    let _guard = lock(&dir.join("request.lock"))?;
    let worker_probe = lock(&dir.join("worker.lock"))?;
    drop(worker_probe);
    let old = status(&root)?;
    if old["status"] == "working"
        && backend::now() - old["startedAt"].as_f64().unwrap_or(0.0) < 120.0
    {
        return Err("A restart is already running. Wait for its result.".into());
    }
    if backend::now() - old["finishedAt"].as_f64().unwrap_or(0.0) < 10.0 {
        return Err("Wait a few seconds before restarting again.".into());
    }
    let review = inspect(&root, url)?;
    if review["snapshot"] != snapshot {
        return Err("The active agents changed. Review the restart again.".into());
    }
    let closing = mode == "terminals" || review["preserveTerminals"] != true;
    if closing && review["liveCount"].as_u64().unwrap_or(0) > 0 {
        if !confirmed {
            return Err("Confirm that running commands and subagents may be interrupted before resuming the saved chats.".into());
        }
        if !arr(&review["problems"]).is_empty() {
            return Err(
                "Some chats cannot be resumed. Fix the listed UUID or directory problems first."
                    .into(),
            );
        }
    }
    let id = backend::uuid();
    let job = json!({"id":id,"status":"working","phase":"starting","message":"Starting restart helper…","startedAt":backend::now(),"url":url,"mode":mode,"snapshot":snapshot,"review":review,"sessions":review["sessions"],"resumed":[],"errors":[]});
    let path = dir.join(format!("{id}.json"));
    write(&path, &job)?;
    write(&dir.join("current.json"), &job)?;
    let mut command = Command::new(latest_binary()?);
    command
        .arg("--restart-dashboard")
        .arg("--home")
        .arg(&root)
        .arg("--job")
        .arg(&id)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match detached(&mut command) {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => {
            let mut failed = job;
            failed["status"] = json!("failed");
            failed["message"] = json!(e);
            failed["finishedAt"] = json!(backend::now());
            write(&path, &failed)?;
            write(&dir.join("current.json"), &failed)?;
            return Err("Could not start the restart helper.".into());
        }
    }
    Ok(job)
}
fn patch(root: &Path, id: &str, p: Value) -> Result<Value> {
    let dir = directory(root)?;
    let path = dir.join(format!("{id}.json"));
    let mut job = read(&path)?;
    for (k, v) in p.as_object().ok_or("Invalid restart patch.")? {
        job[k] = v.clone();
    }
    write(&path, &job)?;
    write(&dir.join("current.json"), &job)?;
    Ok(job)
}
fn phase(root: &Path, id: &str, name: &str, message: &str) -> Result<()> {
    patch(root, id, json!({"phase":name,"message":message}))?;
    Ok(())
}
fn stop(pid: u32, mode: &str, root: &Path) -> Result<()> {
    if !matches_process(pid, mode, root) {
        return Err("The target process changed. No signal was sent.".into());
    }
    if unsafe { libc::kill(pid as i32, libc::SIGTERM) } != 0 {
        return Err("Could not stop the verified truffle process.".into());
    }
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        if !matches_process(pid, mode, root) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(80));
    }
    Err("The process did not stop. It was not force-killed.".into())
}
fn start_service(root: &Path, url: &str) -> Result<()> {
    if service_pid(root, url)?.is_some() {
        return Ok(());
    }
    let logs = root.join("data/logs");
    io(std::fs::create_dir_all(&logs))?;
    let path = logs.join("service.log");
    if path.metadata().is_ok_and(|m| m.len() > 2 * 1024 * 1024) {
        let _ = std::fs::rename(&path, logs.join("service.log.1"));
    }
    let log = io(OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path))?;
    let err = io(log.try_clone())?;
    let mut command = Command::new(io(std::env::current_exe())?);
    command
        .arg("--serve")
        .arg("--home")
        .arg(root)
        .arg("--data")
        .arg(data(root))
        .arg("--port")
        .arg(port(url)?.to_string())
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err));
    command.env(
        "PATH",
        format!(
            "{}/.local/bin:/opt/homebrew/bin:/usr/local/bin:{}",
            backend::home().display(),
            std::env::var("PATH").unwrap_or_default()
        ),
    );
    let mut child = detached(&mut command)?;
    let until = Instant::now() + Duration::from_secs(30);
    while Instant::now() < until {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("The replacement service exited. See data/logs/service.log.".into());
        }
        if runtime(root, url).is_ok_and(|r| r["pid"] == child.id())
            && LocalHttpService::new(url)?.health().is_ok()
        {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Err("The replacement service is still starting. See data/logs/service.log.".into())
}
fn resume(root: &Path, url: &str, records: &[Value]) -> Vec<String> {
    let mut failures = vec![];
    let current = read_state(root).unwrap_or(json!({}));
    let sessions = arr(&current["sessions"]);
    for record in records {
        if !sessions.iter().any(|s| {
            s["id"] == record["id"]
                && s["agent"] == record["agent"]
                && s["sessionId"] == record["sessionId"]
                && s["cwd"] == record["cwd"]
        }) {
            failures.push(format!(
                "{}: saved association changed; resume from History.",
                text(record, "title")
            ));
            continue;
        }
        if let Err(error) = api(url, "/api/session/open", Some(json!({"id":record["id"]}))) {
            failures.push(format!("{}: {error}", text(record, "title")));
        }
    }
    failures
}
pub fn run(root: &Path, id: &str) -> Result<()> {
    backend::sid(id).map_err(|e| e.message)?;
    let root = io(root.canonicalize())?;
    let dir = directory(&root)?;
    let _guard = lock(&dir.join("worker.lock"))?;
    let job = read(&dir.join(format!("{id}.json")))?;
    let url = text(&job, "url");
    let mode = text(&job, "mode");
    if !["service", "terminals"].contains(&mode) {
        return Err("Invalid restart mode.".into());
    }
    let records = arr(&job["sessions"]);
    let mut touched = false;
    let mut interrupted = false;
    let result = (|| -> Result<Vec<String>> {
        phase(&root, id, "checking", "Checking active terminals…")?;
        let review = inspect(&root, url)?;
        if review["snapshot"] != job["snapshot"] {
            return Err("The active agents changed before the restart. Nothing was stopped; review and retry.".into());
        }
        let preserve = mode == "service" && review["preserveTerminals"] == true;
        if !preserve && review["preserveTerminals"] != true {
            phase(
                &root,
                id,
                "saving",
                "Closing old service-owned terminals before resuming their saved chats…",
            )?;
            for record in &records {
                touched = true;
                interrupted = true;
                api(
                    url,
                    "/api/terminal/close",
                    Some(json!({"id":record["id"],"generation":record["generation"]})),
                )?;
                touched = true;
            }
            let remaining = api(url, "/api/agents", None)?;
            if arr(&remaining["candidates"])
                .iter()
                .any(|r| r["live"] == true)
            {
                return Err("Another terminal opened during restart. The service was left running; review and retry.".into());
            }
        }
        phase(&root, id, "service", "Restarting dashboard service…")?;
        if let Some(pid) = review["servicePid"].as_u64() {
            touched = true;
            stop(pid as u32, "--serve", &root)?;
        }
        if mode == "terminals" {
            if let Some(host) = daemon(&root)? {
                let actual = arr(&daemon_request(&root, json!({"op":"inventory"}))?);
                if actual.iter().filter(|r| r["live"] == true).any(|r| {
                    !records.iter().any(|saved| {
                        saved["id"] == r["recordId"] && saved["generation"] == r["generation"]
                    })
                }) {
                    return Err("Another terminal opened before shutdown. The terminal daemon was left running.".into());
                }
                phase(&root, id, "terminals", "Restarting terminal daemon…")?;
                for candidate in actual.iter().filter(|r| r["live"] == true) {
                    touched = true;
                    interrupted = true;
                    daemon_request(
                        &root,
                        json!({"op":"close","id":candidate["recordId"],"generation":candidate["generation"]}),
                    )?;
                    touched = true;
                }
                stop(
                    host["pid"].as_u64().ok_or("Invalid terminal daemon PID.")? as u32,
                    "--terminal-daemon",
                    &root,
                )?;
            }
        }
        start_service(&root, url)?;
        if preserve {
            Ok(vec![])
        } else {
            phase(
                &root,
                id,
                "resuming",
                "Resuming saved conversations in their original folders…",
            )?;
            Ok(resume(&root, url, &records))
        }
    })();
    match result {
        Ok(errors) => {
            let success = errors.is_empty();
            patch(
                &root,
                id,
                json!({"status":if success{"succeeded"}else{"failed"},"phase":"finished","finishedAt":backend::now(),"errors":errors,"message":if success{if mode=="service"&&job["review"]["preserveTerminals"]==true{"Service restarted; running terminals were preserved."}else{"Restarted; saved conversations were reopened in their original folders."}}else{"Restarted, but some chats need attention. Open History for those listed below."}}),
            )?;
            Ok(())
        }
        Err(error) => {
            let mut errors = vec![error.clone()];
            if touched {
                if let Err(e) = start_service(&root, url) {
                    errors.push(e);
                } else if interrupted {
                    errors.extend(resume(&root, url, &records));
                }
            }
            patch(
                &root,
                id,
                json!({"status":"failed","phase":"finished","finishedAt":backend::now(),"errors":errors,"message":error}),
            )?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn process_profile_arguments_do_not_match_a_different_prefix() {
        assert!(argument_matches(
            "/app/truffle-desktop --serve --home /code/mydash --port 8765",
            "--home",
            "/code/mydash"
        ));
        assert!(argument_matches(
            "/app/truffle-desktop --serve --home /code/my dash",
            "--home",
            "/code/my dash"
        ));
        assert!(!argument_matches(
            "/app/truffle-desktop --serve --home /code/mydashboard",
            "--home",
            "/code/mydash"
        ));
        assert!(!argument_matches(
            "/app/truffle-desktop --serve --other--home /code/mydash",
            "--home",
            "/code/mydash"
        ));
    }
}
