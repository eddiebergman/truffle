//! Native registration and lifecycle client for the local agent bridge.
use super::{expand, home, json, now, safe_url, sid, terminal, text, uuid, Error, Result, Value};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
const LIMIT: usize = 128 * 1024;
const COMMANDS: &[&str] = &[
    "command",
    "register",
    "settings",
    "artifact",
    "checkpoint",
    "briefing",
    "status",
    "worktree",
    "resource",
    "context",
    "review",
    "ci",
    "hook",
];
pub fn helper(root: &Path) -> Result<String> {
    Ok(format!(
        "{} --home {}",
        terminal::quote(&std::env::current_exe()?.to_string_lossy()),
        terminal::quote(&root.to_string_lossy())
    ))
}
struct Options {
    values: BTreeMap<String, Vec<String>>,
    flags: BTreeSet<String>,
}
impl Options {
    fn parse(args: &[String]) -> Result<Self> {
        let mut out = Self {
            values: BTreeMap::new(),
            flags: BTreeSet::new(),
        };
        let mut index = 0;
        while index < args.len() {
            let key = args[index]
                .strip_prefix("--")
                .ok_or_else(|| Error::new("Use named command options."))?;
            if [
                "show",
                "primary",
                "replace",
                "help",
                "inbox",
                "skill-output",
            ]
            .contains(&key)
            {
                if !out.flags.insert(key.into()) {
                    return Err(Error::new("Duplicate command flag."));
                }
                index += 1;
            } else {
                let value = args
                    .get(index + 1)
                    .filter(|x| !x.starts_with("--"))
                    .ok_or_else(|| Error::new("Command option needs a value."))?;
                out.values
                    .entry(key.into())
                    .or_default()
                    .push(value.clone());
                index += 2;
            }
        }
        Ok(out)
    }
    fn get(&self, key: &str) -> &str {
        self.values
            .get(key)
            .and_then(|v| v.first())
            .map_or("", String::as_str)
    }
    fn required(&self, key: &str) -> Result<&str> {
        let value = self.get(key);
        if value.is_empty() {
            Err(Error::new(format!("Supply --{key}.")))
        } else {
            Ok(value)
        }
    }
    fn many(&self, key: &str) -> Vec<String> {
        self.values.get(key).cloned().unwrap_or_default()
    }
    fn has(&self, key: &str) -> bool {
        self.flags.contains(key)
    }
    fn validate(&self, allowed: &[&str]) -> Result<()> {
        for (key, values) in &self.values {
            if key != "home" && !allowed.contains(&key.as_str()) {
                return Err(Error::new(format!("Unknown option --{key}.")));
            }
            if values.len() > 1 && !matches!(key.as_str(), "url" | "item") {
                return Err(Error::new(format!("Supply --{key} once.")));
            }
        }
        for key in &self.flags {
            if !["help", "inbox"].contains(&key.as_str()) && !allowed.contains(&key.as_str()) {
                return Err(Error::new(format!("Unknown flag --{key}.")));
            }
        }
        Ok(())
    }
}
fn read(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(Error::new("Input exceeds the command size limit."));
    }
    Ok(bytes)
}
fn json_file(options: &Options, limit: usize) -> Result<Value> {
    Ok(serde_json::from_slice(&read(
        &expand(options.required("file")?),
        limit,
    )?)?)
}
fn path(value: &str) -> Result<String> {
    Ok(fs::canonicalize(expand(value))?.to_string_lossy().into())
}
fn require_context_revision(input: &Value) -> Result<()> {
    if !input["revision"].as_str().is_some_and(|revision| {
        revision.len() == 64 && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) {
        return Err(Error::new("Replacement JSON requires the captured revision from context --record ID --show. Keep that revision while editing; on conflict, re-read context and reapply your intended edits."));
    }
    Ok(())
}
fn payload(command: &str, o: &Options) -> Result<Value> {
    let mut p = json!({});
    if command != "register" && command != "settings" && command != "command" {
        p["id"] = json!(sid(o.required("record")?)?);
    }
    match command {
        "command" => {
            o.validate(&["name", "file"])?;
            p["operation"] = json!("command");
            p["name"] = json!(o.required("name")?);
            p["args"] = if o.get("file").is_empty() {
                json!({})
            } else {
                json_file(o, 64 * 1024)?
            };
        }
        "settings" => {
            o.validate(&["show", "file", "revision", "content-hash"])?;
            if o.has("show") {
                if !o.get("file").is_empty() {
                    return Err(Error::new("Read settings separately from updates."));
                }
                p["operation"] = json!("settings-read");
            } else {
                p["operation"] = json!("settings-apply");
                p["patch"] = json_file(o, 64 * 1024)?;
                p["revision"] = json!(o
                    .required("revision")?
                    .parse::<u64>()
                    .map_err(|_| Error::new("Supply the current settings revision."))?);
                let hash=o.required("content-hash").map_err(|_|Error::new("Read settings --show, then supply its revision and --content-hash with your patch."))?;
                if hash.len() != 64
                    || !hash
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return Err(Error::new("Supply --content-hash from settings --show."));
                }
                p["contentHash"] = json!(hash);
            }
        }
        "register" => {
            o.validate(&[
                "agent",
                "session",
                "record",
                "title",
                "cwd",
                "url",
                "item",
                "project",
                "destination",
            ])?;
            let agent = o.required("agent")?;
            if !["claude", "codex"].contains(&agent) {
                return Err(Error::new("Choose Claude or Codex."));
            }
            let variable = if agent == "claude" {
                "CLAUDE_CODE_SESSION_ID"
            } else {
                "CODEX_THREAD_ID"
            };
            let session = if o.get("session").is_empty() {
                std::env::var(variable).unwrap_or_default()
            } else {
                o.get("session").into()
            };
            let cwd = if o.get("cwd").is_empty() {
                std::env::current_dir()?.to_string_lossy().into()
            } else {
                path(o.get("cwd"))?
            };
            p = json!({"agent":agent,"sessionId":sid(&session)?,"cwd":cwd,"urls":o.many("url"),"itemIds":o.many("item")});
            for (key, field) in [
                ("record", "id"),
                ("title", "title"),
                ("project", "project"),
                ("destination", "destination"),
            ] {
                if !o.get(key).is_empty() {
                    p[field] = json!(o.get(key));
                }
            }
        }
        "review" => {
            o.validate(&[
                "record",
                "request",
                "state",
                "pass",
                "file",
                "show",
                "reviewed-revision",
            ])?;
            if o.has("show")
                && (!o.get("file").is_empty()
                    || !o.get("state").is_empty()
                    || !o.get("pass").is_empty())
            {
                return Err(Error::new("Read the draft separately from review updates."));
            }
            if !o.has("show") && o.get("file").is_empty() && o.get("state").is_empty() {
                return Err(Error::new("Review needs --show, --file, or --state."));
            }
            p["operation"] = json!("review");
            for (key, field) in [
                ("request", "requestId"),
                ("state", "state"),
                ("pass", "pass"),
            ] {
                if !o.get(key).is_empty() {
                    p[field] = json!(o.get(key));
                }
            }
            if !o.get("state").is_empty() || !o.get("pass").is_empty() {
                sid(o.required("request")?)?;
            }
            if !o.get("file").is_empty() {
                p["report"] = json_file(o, 96 * 1024)?;
            }
            if !o.get("reviewed-revision").is_empty() {
                p["reviewedRevision"] = json!(o
                    .get("reviewed-revision")
                    .parse::<u64>()
                    .map_err(|_| Error::new("Invalid review revision."))?);
            }
        }
        "context" => {
            o.validate(&["record", "file", "show"])?;
            if o.has("show") != o.get("file").is_empty() {
                return Err(Error::new("Choose either --show or --file."));
            }
            p["operation"] = json!(if o.has("show") {
                "context-read"
            } else {
                "context-replace"
            });
            if !o.has("show") {
                p["context"] = json_file(o, 96 * 1024)?;
                require_context_revision(&p["context"])?;
            }
        }
        "resource" => {
            o.validate(&["record", "file", "replace"])?;
            let mut report = json_file(o, 64 * 1024)?;
            if !report["resources"].is_array() {
                return Err(Error::new("Resource report needs a resources array."));
            }
            if o.has("replace") {
                require_context_revision(&report)?;
            }
            report["replace"] = json!(o.has("replace"));
            p["operation"] = json!("resources");
            p["report"] = report;
        }
        "worktree" => {
            o.validate(&["record", "path", "url", "item", "primary"])?;
            let cwd = std::env::current_dir()?;
            p["operation"] = json!("worktree");
            p["path"] = json!(path(if o.get("path").is_empty() {
                cwd.to_str().ok_or_else(Error::io)?
            } else {
                o.get("path")
            })?);
            p["urls"] = json!(o.many("url"));
            p["itemIds"] = json!(o.many("item"));
            p["primary"] = json!(o.has("primary"));
        }
        "status" => {
            o.validate(&["record", "state", "subagents", "reason"])?;
            p["operation"] = json!("runtime");
            p["state"] = json!(o.required("state")?);
            if !o.get("subagents").is_empty() {
                p["subagents"] = json!(o
                    .get("subagents")
                    .parse::<u64>()
                    .map_err(|_| Error::new("Subagent count must be nonnegative."))?);
            }
            if !o.get("reason").is_empty() {
                p["reason"] = json!(o.get("reason"));
            }
        }
        "ci" => {
            o.validate(&["record", "item", "request", "state"])?;
            p["operation"] = json!("ci-investigation");
            p["itemId"] = json!(o.required("item")?);
            p["requestId"] = json!(sid(o.required("request")?)?);
            p["state"] = json!(o.required("state")?);
        }
        "artifact" => {
            o.validate(&["record", "file", "url", "title", "skill-output"])?;
            if o.has("skill-output") {
                p["skillOutput"] = json!(true);
            }
            if o.get("file").is_empty() == o.get("url").is_empty() {
                return Err(Error::new("Choose a file or URL."));
            }
            p["operation"] = json!("save-artifact");
            if o.get("file").is_empty() {
                p["url"] = json!(safe_url(o.get("url"))?);
            } else {
                p["path"] = json!(path(o.get("file"))?);
            }
            if !o.get("title").is_empty() {
                p["title"] = json!(o.get("title"));
            }
        }
        "checkpoint" | "briefing" => {
            o.validate(&["record", "file"])?;
            let bytes = read(&expand(o.required("file")?), 96000)?;
            if command == "briefing" {
                p["operation"] = json!("save-briefing");
                p["body"] =
                    json!(String::from_utf8(bytes)
                        .map_err(|_| Error::new("Report must be UTF-8."))?);
            } else {
                let report: Value = serde_json::from_slice(&bytes)?;
                let object = report
                    .as_object()
                    .ok_or_else(|| Error::new("Checkpoint must be an object."))?;
                if object
                    .keys()
                    .any(|k| !["done", "blocked", "next"].contains(&k.as_str()))
                {
                    return Err(Error::new(
                        "Checkpoint supports done, blocked, and next only.",
                    ));
                }
                p["operation"] = json!("checkpoint");
                for (k, v) in object {
                    p[k] = v.clone();
                }
            }
        }
        _ => return Err(Error::new("Unknown agent command.")),
    }
    Ok(p)
}
fn runtime(root: &Path) -> Result<Value> {
    let value: Value = serde_json::from_slice(&read(&root.join("data/runtime.json"), 16384)?)?;
    let url = reqwest::Url::parse(text(&value, "url"))
        .map_err(|_| Error::new("Invalid local runtime address."))?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
        || text(&value, "token").is_empty()
    {
        return Err(Error::new("Invalid local runtime address."));
    }
    Ok(value)
}
fn private_create(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    Ok(())
}
fn enqueue(runtime: &Value, payload: &Value, reply: bool) -> Result<(PathBuf, String)> {
    let inbox = PathBuf::from(text(runtime, "inbox"));
    let meta = fs::symlink_metadata(&inbox)?;
    if !meta.is_dir()
        || meta.uid() != unsafe { libc::getuid() }
        || meta.mode() & 0o077 != 0
        || inbox.parent().and_then(|p| p.canonicalize().ok())
            != Path::new("/tmp").canonicalize().ok()
        || !inbox
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .starts_with("truffle-agent-")
    {
        return Err(Error::new("Invalid local registration inbox."));
    }
    let nonce = uuid();
    let data = json!({"nonce":nonce,"time":now(),"payload":payload,"reply":reply});
    let mut mac = Hmac::<Sha256>::new_from_slice(text(runtime, "token").as_bytes())
        .map_err(|_| Error::io())?;
    mac.update(super::http::canonical(&data).as_bytes());
    let signature = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let bytes = serde_json::to_vec(&json!({"data":data,"signature":signature}))?;
    if bytes.len() > LIMIT {
        return Err(Error::new("Registration request exceeds 128 KiB."));
    }
    let temp = inbox.join(format!("{nonce}.tmp"));
    private_create(&temp, &bytes)?;
    fs::rename(temp, inbox.join(format!("{nonce}.request.json")))?;
    Ok((inbox, nonce))
}
fn wait_inbox(inbox: &Path, nonce: &str) -> Result<Value> {
    let response = inbox.join(format!("{nonce}.result.json"));
    let request = inbox.join(format!("{nonce}.request.json"));
    let start = Instant::now();
    let result = (|| {
        while start.elapsed() < Duration::from_secs(90) {
            match fs::symlink_metadata(&response) {
                Ok(meta) => {
                    if !meta.is_file() || meta.uid() != unsafe { libc::getuid() } {
                        return Err(Error::new("Invalid registration response."));
                    }
                    let value: Value = serde_json::from_slice(&read(&response, 1024 * 1024)?)?;
                    if let Some(error) = value["error"].as_str() {
                        return Err(Error::new(error));
                    }
                    return Ok(value);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(Error::new(
            "The local service did not acknowledge this request. Check it before retrying.",
        ))
    })();
    let _ = fs::remove_file(request);
    let _ = fs::remove_file(response);
    result
}
fn send(runtime: &Value, payload: &Value, inbox_only: bool, hook: bool) -> Result<Value> {
    if !inbox_only {
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(if hook { 1500 } else { 120000 }))
            .build()
            .map_err(|_| Error::io())?;
        match client
            .post(format!(
                "{}/api/agent/register",
                text(runtime, "url").trim_end_matches('/')
            ))
            .bearer_auth(text(runtime, "token"))
            .json(payload)
            .send()
        {
            Ok(response) => {
                let status = response.status();
                let mut bytes = Vec::new();
                response.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
                if bytes.len() > 1024 * 1024 {
                    return Err(Error::new("Local response exceeds 1 MiB."));
                }
                let value: Value = serde_json::from_slice(&bytes)?;
                if !status.is_success() {
                    return Err(Error::new(
                        value["error"].as_str().unwrap_or("Registration failed."),
                    ));
                }
                return Ok(value);
            }
            Err(error) if error.is_connect() => {}
            Err(_) => {
                return Err(Error::new(
                    "The request was not acknowledged. Read the current state before retrying.",
                ))
            }
        }
    }
    let (inbox, nonce) = enqueue(runtime, payload, !hook)?;
    if hook {
        Ok(json!({}))
    } else {
        wait_inbox(&inbox, &nonce)
    }
}
pub fn activity_snapshot(data: &Value) -> Value {
    let mut result = json!({});
    if let Some(tasks) = data["background_tasks"].as_array() {
        let mut seen = BTreeSet::new();
        let mut counts = BTreeMap::<String, u64>::new();
        for (index, task) in tasks.iter().take(1000).enumerate() {
            if !task.is_object()
                || [
                    "completed",
                    "failed",
                    "cancelled",
                    "killed",
                    "stopped",
                    "done",
                ]
                .contains(&text(task, "status"))
            {
                continue;
            }
            let kind = task["type"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 40)
                .unwrap_or("other");
            let identity = task
                .get("id")
                .map_or_else(|| index.to_string(), Value::to_string);
            if seen.insert((kind.to_owned(), identity)) {
                *counts.entry(kind.into()).or_default() += 1;
            }
        }
        result["backgroundCounts"] = json!(counts);
    }
    if let Some(crons) = data["session_crons"].as_array() {
        result["scheduledWakeups"] = json!(crons.len().min(1000));
    }
    result
}
fn hook(o: &Options, root: &Path) -> Result<()> {
    o.validate(&["record", "event"])?;
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(LIMIT as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > LIMIT {
        return Err(Error::new("Hook input exceeds 128 KiB."));
    }
    let data: Value = serde_json::from_slice(&bytes)?;
    let event = o.required("event")?;
    if !text(&data, "agent_id").is_empty()
        && ["SessionStart", "Stop", "UserPromptSubmit"].contains(&event)
    {
        return Ok(());
    }
    let mut payload = json!({"id":sid(o.required("record")?)?,"operation":"runtime-event","event":event,"sessionId":text(&data,"session_id"),"subagentId":if text(&data,"agent_id").is_empty(){text(&data,"subagent_id")}else{text(&data,"agent_id")}});
    if ["Stop", "SubagentStop"].contains(&event) {
        for (k, v) in activity_snapshot(&data).as_object().unwrap() {
            payload[k] = v.clone();
        }
    }
    send(&runtime(root)?, &payload, o.has("inbox"), true)?;
    Ok(())
}
fn shown_result<'a>(command: &str, result: &'a Value) -> &'a Value {
    &result[if command == "review" {
        "review"
    } else {
        "context"
    }]
}
fn command_help(command: &str) -> String {
    format!("{command}: --record UUID [--home DIRECTORY] [--file PATH] [--state STATE] [--show]\nregister: --agent claude|codex [--session UUID] [--cwd PATH] [--url URL] [--item ID]\nreview/ci: --request UUID [--pass NAME] [--reviewed-revision NUMBER]\ncommand: --name COMMAND [--file ARGUMENTS_JSON]\nsettings: --show OR --file PATCH_JSON --revision NUMBER --content-hash HASH (routine settings only)\ncontext: --record UUID --show OR --file CONTEXT_JSON (replacement requires its captured revision)\nresource: --record UUID --file REPORT_JSON [--replace]; --replace requires report.revision from context --show\nReplacement lists: omitted stays unchanged; [] clears dashboard links only, never external resources/files. On conflict re-read context and reapply intended edits; do not substitute a fresh revision into the stale file.\nUse --inbox to use the private signed local inbox.")
}
pub fn run(args: &[String]) -> Option<Result<()>> {
    let mut index = 1;
    let mut prefix = Vec::new();
    if args.get(index).is_some_and(|x| x == "--home") {
        prefix.extend(args.get(index..index + 2)?.iter().cloned());
        index += 2;
    }
    let command = args.get(index)?;
    if command == "--help" {
        println!("Truffle: {}\nUse COMMAND --help for options. Discover app commands with: command --name commands.list\nService: --serve --home DIRECTORY", COMMANDS.join(", "));
        return Some(Ok(()));
    }
    if !COMMANDS.contains(&command.as_str()) {
        return None;
    }
    prefix.extend(args[index + 1..].iter().cloned());
    let result = (|| {
        let options = Options::parse(&prefix)?;
        if options.has("help") {
            println!("{}", command_help(command));
            return Ok(());
        }
        let root = if options.get("home").is_empty() {
            std::env::var_os("TRUFFLE_HOME")
                .map_or_else(|| home().join("code/mydash"), PathBuf::from)
        } else {
            expand(options.get("home"))
        };
        if command == "hook" {
            return hook(&options, &root);
        }
        let result = send(
            &runtime(&root)?,
            &payload(command, &options)?,
            options.has("inbox"),
            false,
        )?;
        if command == "command" {
            println!(
                "{}",
                serde_json::to_string_pretty(&result["commandResult"])?
            );
        } else if command == "settings" {
            println!("{}", serde_json::to_string_pretty(&result["settings"])?);
        } else if options.has("show") {
            println!(
                "{}",
                serde_json::to_string_pretty(shown_result(command, &result))?
            );
        } else {
            println!(
                "Saved to truffle. {}#session={}",
                text(&runtime(&root)?, "url").trim_end_matches('/'),
                text(&result, "id")
            );
        }
        Ok(())
    })();
    Some(if command == "hook" { Ok(()) } else { result })
}

#[cfg(test)]
mod tests;
