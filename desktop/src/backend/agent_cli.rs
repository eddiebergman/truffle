//! Native registration and lifecycle client for the local agent bridge.
use super::*;
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
                .ok_or(Error::new("Use named command options."))?;
            if ["show", "primary", "replace", "help", "inbox", "skill-output"].contains(&key) {
                if !out.flags.insert(key.into()) {
                    return Err(Error::new("Duplicate command flag."));
                }
                index += 1;
            } else {
                let value = args
                    .get(index + 1)
                    .filter(|x| !x.starts_with("--"))
                    .ok_or(Error::new("Command option needs a value."))?;
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
            .map(String::as_str)
            .unwrap_or("")
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
fn require_context_revision(input:&Value)->Result<()> {
    if !input["revision"].as_str().is_some_and(|revision|revision.len()==64&&revision.bytes().all(|byte|byte.is_ascii_hexdigit())) {
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
        "command"=>{o.validate(&["name","file"])?;p["operation"]=json!("command");p["name"]=json!(o.required("name")?);p["args"]=if o.get("file").is_empty(){json!({})}else{json_file(o,64*1024)?};}
        "settings" => {
            o.validate(&["show","file","revision","content-hash"])?;
            if o.has("show") { if !o.get("file").is_empty(){return Err(Error::new("Read settings separately from updates."));}p["operation"]=json!("settings-read"); } else {p["operation"]=json!("settings-apply");p["patch"]=json_file(o,64*1024)?;p["revision"]=json!(o.required("revision")?.parse::<u64>().map_err(|_|Error::new("Supply the current settings revision."))?);let hash=o.required("content-hash").map_err(|_|Error::new("Read settings --show, then supply its revision and --content-hash with your patch."))?;if hash.len()!=64||!hash.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)){return Err(Error::new("Supply --content-hash from settings --show."));}p["contentHash"]=json!(hash);}
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
            if o.has("show") == !o.get("file").is_empty() {
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
            if o.has("replace"){require_context_revision(&report)?;}
            report["replace"] = json!(o.has("replace"));
            p["operation"] = json!("resources");
            p["report"] = report;
        }
        "worktree" => {
            o.validate(&["record", "path", "url", "item", "primary"])?;
            let cwd = std::env::current_dir()?;
            p["operation"] = json!("worktree");
            p["path"] = json!(path(if o.get("path").is_empty() {
                cwd.to_str().ok_or(Error::io())?
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
            if o.has("skill-output"){p["skillOutput"]=json!(true);}
            if o.get("file").is_empty() == o.get("url").is_empty() {
                return Err(Error::new("Choose a file or URL."));
            }
            p["operation"] = json!("save-artifact");
            if !o.get("file").is_empty() {
                p["path"] = json!(path(o.get("file"))?);
            } else {
                p["url"] = json!(safe_url(o.get("url"))?);
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
                    .ok_or(Error::new("Checkpoint must be an object."))?;
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
            .starts_with("mydash-agent-")
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
                .map(Value::to_string)
                .unwrap_or_else(|| index.to_string());
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
fn shown_result<'a>(command:&str,result:&'a Value)->&'a Value {
    &result[if command=="review"{"review"}else{"context"}]
}
fn command_help(command:&str)->String {
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
            println!("{}",command_help(command));
            return Ok(());
        }
        let root = if options.get("home").is_empty() {
            std::env::var_os("MYDASH_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home().join("code/mydash"))
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
        if command=="command" {println!("{}",serde_json::to_string_pretty(&result["commandResult"])?);} else if command=="settings" { println!("{}",serde_json::to_string_pretty(&result["settings"])?); } else if options.has("show") {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    shown_result(command,&result)
                )?
            );
        } else {
            println!(
                "Saved to mydash. {}#session={}",
                text(&runtime(&root)?, "url").trim_end_matches('/'),
                text(&result, "id")
            );
        }
        Ok(())
    })();
    Some(if command == "hook" { Ok(()) } else { result })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(args: &[&str]) -> Options {
        Options::parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
    }
    #[test]
    fn replacement_payloads_require_and_preserve_the_captured_revision_without_new_flags(){
        let temp=tempfile::tempdir().unwrap();let file=temp.path().join("context.json");let path=file.to_str().unwrap();let id=uuid();let revision="aB".repeat(32);
        let context=json!({"revision":revision,"resources":[],"itemIds":[],"artifactIds":[]});fs::write(&file,serde_json::to_vec(&context).unwrap()).unwrap();
        let request=payload("context",&options(&["--record",&id,"--file",path])).unwrap();assert_eq!(request["context"],context);assert_eq!(request["operation"],"context-replace");
        let request=payload("resource",&options(&["--record",&id,"--file",path,"--replace"])).unwrap();assert_eq!(request["report"]["revision"],revision);assert_eq!(request["report"]["resources"],json!([]));assert_eq!(request["report"]["replace"],true);
        assert!(payload("context",&options(&["--record",&id,"--file",path,"--revision",&revision])).is_err());assert!(payload("resource",&options(&["--record",&id,"--file",path,"--replace","--revision",&revision])).is_err());
        for revision in [Value::Null,json!(7),json!(""),json!("fresh"),json!("g".repeat(64))]{fs::write(&file,serde_json::to_vec(&json!({"revision":revision,"resources":[]})).unwrap()).unwrap();for command in ["context","resource"]{let mut args=vec!["--record",&id,"--file",path];if command=="resource"{args.push("--replace");}let error=payload(command,&options(&args)).unwrap_err();assert!(error.message.contains("captured revision"));assert!(error.message.contains("reapply"));}}
        fs::write(&file,b"{\"resources\":[],\"replace\":true}").unwrap();let additive=payload("resource",&options(&["--record",&id,"--file",path])).unwrap();assert_eq!(additive["report"]["replace"],false);assert!(additive["report"].get("revision").is_none());
        let shown=payload("context",&options(&["--record",&id,"--show"])).unwrap();assert_eq!(shown["operation"],"context-read");assert!(shown.get("context").is_none());
        let help=command_help("resource");assert!(help.contains("report.revision from context --show"));assert!(help.contains("re-read context and reapply"));assert!(help.contains("[] clears dashboard links only"));
    }
    #[test]
    fn context_show_prints_the_owner_context_for_direct_edit_and_resubmit(){
        let temp=tempfile::tempdir().unwrap();let repo=Repository::open(temp.path().to_path_buf(),None,true).unwrap();let record=repo.transaction(|state|super::super::workflows::save_session(state,&json!({"agent":"codex","title":"Context fixture","cwd":temp.path(),"itemIds":[]}))).unwrap();let id=text(&record,"id");
        let owner=super::super::workflows::register(&repo,&json!({"operation":"context-read","id":id})).unwrap();let reply=super::super::workflows::registration_reply(&owner);let shown=shown_result("context",&reply);assert_eq!(shown,&owner["context"]);assert!(shown.get("id").is_none());assert!(shown.get("context").is_none());assert_eq!(shown["revision"].as_str().unwrap().len(),64);
        let file=temp.path().join("context.json");fs::write(&file,serde_json::to_vec(shown).unwrap()).unwrap();let request=payload("context",&options(&["--record",id,"--file",file.to_str().unwrap()])).unwrap();assert_eq!(request["context"],*shown);super::super::workflows::register(&repo,&request).unwrap();
    }
    #[test]
    fn settings_updates_carry_the_explicit_captured_content_hash() {
        let temp=tempfile::tempdir().unwrap();let file=temp.path().join("patch.json");std::fs::write(&file,r#"{"theme":"light"}"#).unwrap();let file=file.to_str().unwrap();let hash="a".repeat(64);
        let request=payload("settings",&options(&["--file",file,"--revision","7","--content-hash",&hash])).unwrap();
        assert_eq!(request["revision"],7);assert_eq!(request["contentHash"],hash);assert_eq!(request["patch"],json!({"theme":"light"}));
        assert!(payload("settings",&options(&["--file",file,"--revision","7"])).unwrap_err().message.contains("--content-hash"));
        assert!(payload("settings",&options(&["--file",file,"--revision","7","--content-hash","invalid"])).is_err());
    }
    #[test]
    fn native_payloads_preserve_ids_repeated_links_and_validate_input() {
        let id = uuid();
        let session = uuid();
        let o = options(&[
            "--agent",
            "claude",
            "--session",
            &session,
            "--record",
            &id,
            "--url",
            "https://example.com/one",
            "--url",
            "https://example.com/two",
        ]);
        let value = payload("register", &o).unwrap();
        assert_eq!(value["sessionId"], session);
        assert_eq!(arr(&value["urls"]).len(), 2);
        assert!(payload(
            "register",
            &options(&["--agent", "unknown", "--session", &session])
        )
        .is_err());
        assert!(payload(
            "review",
            &options(&["--record", &id, "--state", "complete"])
        )
        .is_err());
        assert!(payload("context", &options(&["--record", &id])).is_err());
        assert!(payload(
            "context",
            &options(&["--record", &id, "--show", "--file", "missing"])
        )
        .is_err());
        assert!(payload(
            "status",
            &options(&["--record", &id, "--state", "working", "--subagents", "-1"])
        )
        .is_err());
        assert!(payload(
            "status",
            &options(&["--record", &id, "--state", "working", "--typo", "ignored"])
        )
        .is_err());
        assert!(payload(
            "artifact",
            &options(&[
                "--record",
                &id,
                "--file",
                "a",
                "--url",
                "https://example.com"
            ])
        )
        .is_err());
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("report.json");
        fs::write(
            &file,
            b"{\"comments\":[],\"overall\":{\"author\":\"Agent\",\"text\":\"No findings\"}}",
        )
        .unwrap();
        let report = payload(
            "review",
            &options(&[
                "--record",
                &id,
                "--request",
                &session,
                "--pass",
                "custom-reviewer",
                "--file",
                file.to_str().unwrap(),
                "--state",
                "complete",
            ]),
        )
        .unwrap();
        assert_eq!(report["report"]["overall"]["text"], "No findings");
        assert_eq!(report["pass"], "custom-reviewer");
        fs::write(&file, vec![b'x'; 96 * 1024 + 1]).unwrap();
        assert!(payload(
            "review",
            &options(&["--record", &id, "--file", file.to_str().unwrap()])
        )
        .is_err());
    }
    #[test]
    fn hooks_keep_counts_only_and_preserve_unknown_and_empty_snapshots() {
        let data = json!({"background_tasks":[{"id":"s","type":"shell","command":"private"},{"id":"s","type":"shell"},{"id":"m","type":"monitor"},{"id":"done","type":"shell","status":"completed"},null,{"type":[]}],"session_crons":[{"prompt":"private"}]});
        assert_eq!(
            activity_snapshot(&data),
            json!({"backgroundCounts":{"shell":1,"monitor":1,"other":1},"scheduledWakeups":1})
        );
        assert_eq!(activity_snapshot(&json!({})), json!({}));
        assert_eq!(
            activity_snapshot(&json!({"background_tasks":[],"session_crons":[]})),
            json!({"backgroundCounts":{},"scheduledWakeups":0})
        );
        assert_eq!(
            activity_snapshot(&json!({"session_crons":vec![json!({});1100]}))["scheduledWakeups"],
            1000
        );
    }
    #[test]
    fn signed_inbox_roundtrip_preserves_unicode_and_nested_replies_and_hooks_need_no_ack() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        let record = repo
            .transaction(|s| {
                workflows::save_session(
                    s,
                    &json!({"agent":"claude","cwd":temp.path(),"title":"日本語 · café"}),
                )
            })
            .unwrap();
        let id = text(&record, "id");
        let inbox = tempfile::Builder::new()
            .prefix("mydash-agent-")
            .tempdir_in("/tmp")
            .unwrap();
        fs::set_permissions(inbox.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = json!({"token":"private-test-token","inbox":inbox.path()});
        let (root, nonce) =
            enqueue(&runtime, &json!({"id":id,"operation":"review"}), true).unwrap();
        super::super::http::inbox_once(&repo, &root, "private-test-token");
        let reply = wait_inbox(&root, &nonce).unwrap();
        assert_eq!(reply["id"], id);
        assert!(reply["review"]["comments"].is_array());
        let (root, nonce) = enqueue(
            &runtime,
            &json!({"id":id,"operation":"review","report":{"comments":[],"overall":null}}),
            true,
        )
        .unwrap();
        let request = root.join(format!("{nonce}.request.json"));
        let mut envelope: Value = serde_json::from_slice(&fs::read(&request).unwrap()).unwrap();
        envelope["data"]["payload"]["note"] = json!("changed 日本語");
        fs::write(&request, serde_json::to_vec(&envelope).unwrap()).unwrap();
        super::super::http::inbox_once(&repo, &root, "private-test-token");
        assert!(wait_inbox(&root, &nonce).is_err());
        let(root,nonce)=enqueue(&runtime,&json!({"id":id,"operation":"runtime-event","event":"Stop","backgroundCounts":{"shell":0},"sessionId":""}),false).unwrap();
        super::super::http::inbox_once(&repo, &root, "private-test-token");
        assert!(!root.join(format!("{nonce}.result.json")).exists());
        assert!(!root.join(format!("{nonce}.request.json")).exists());
        fs::set_permissions(inbox.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(enqueue(&runtime, &json!({}), true).is_err());
    }
}
