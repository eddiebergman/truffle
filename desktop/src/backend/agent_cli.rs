//! Native registration and lifecycle client for the local agent bridge.
use super::agent_bridge::{
    ArtifactAttachment, Briefing, Checkpoint, CiInvestigation, Command, ContextRead,
    ContextReplace, Operation, Registration, Request, ResourceReport, ReviewUpdate, RuntimeEvent,
    SettingsApply, SettingsRead, StatusReport, WorktreeReport,
};
use super::agents::Agent;
use super::runtime::{Background, Hook};
use super::{expand, home, json, now, safe_url, sid, terminal, text, uuid, Error, Result, Value};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
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
    /// The value of `key`, or `None` when it is missing or empty.
    fn optional(&self, key: &str) -> Option<String> {
        Some(self.get(key))
            .filter(|value| !value.is_empty())
            .map(Into::into)
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
    let request = match command {
        "register" => Request::Register(register_payload(o)?),
        "settings" => Request::Operation(settings_payload(o)?),
        _ => Request::Operation(operation(command, o, sid(o.required("record")?)?)?),
    };
    request.to_json()
}
fn operation(command: &str, o: &Options, id: String) -> Result<Operation> {
    Ok(match command {
        "command" => {
            o.validate(&["record", "name", "file"])?;
            Operation::Command(Command {
                id: Some(id),
                name: o.required("name")?.into(),
                args: if o.get("file").is_empty() {
                    json!({})
                } else {
                    json_file(o, 64 * 1024)?
                },
            })
        }
        "review" => review_payload(o, id)?,
        "context" => context_payload(o, id)?,
        "resource" => resource_payload(o, id)?,
        "worktree" => worktree_payload(o, id)?,
        "status" => status_payload(o, id)?,
        "ci" => {
            o.validate(&["record", "item", "request", "state"])?;
            Operation::CiInvestigation(CiInvestigation {
                id,
                item_id: o.required("item")?.into(),
                request_id: sid(o.required("request")?)?,
                state: o.required("state")?.into(),
            })
        }
        "artifact" => artifact_payload(o, id)?,
        "checkpoint" | "briefing" => report_payload(command, o, id)?,
        _ => return Err(Error::new("Unknown agent command.")),
    })
}
fn settings_payload(o: &Options) -> Result<Operation> {
    o.validate(&["record", "show", "file", "revision", "content-hash"])?;
    let id = o.optional("record").as_deref().map(sid).transpose()?;
    if o.has("show") {
        if !o.get("file").is_empty() {
            return Err(Error::new("Read settings separately from updates."));
        }
        return Ok(Operation::SettingsRead(SettingsRead { id }));
    }
    let patch = json_file(o, 64 * 1024)?;
    let revision = o
        .required("revision")?
        .parse::<u64>()
        .map_err(|_| Error::new("Supply the current settings revision."))?;
    let hash = o.required("content-hash").map_err(|_| {
        Error::new(
            "Read settings --show, then supply its revision and --content-hash with your patch.",
        )
    })?;
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::new("Supply --content-hash from settings --show."));
    }
    Ok(Operation::SettingsApply(SettingsApply {
        id,
        patch,
        revision,
        content_hash: hash.into(),
    }))
}
fn register_payload(o: &Options) -> Result<Registration> {
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
    let agent = Agent::parse(o.required("agent")?)?;
    let session = if o.get("session").is_empty() {
        std::env::var(agent.session_env_var()).unwrap_or_default()
    } else {
        o.get("session").into()
    };
    let cwd = if o.get("cwd").is_empty() {
        std::env::current_dir()?.to_string_lossy().into()
    } else {
        path(o.get("cwd"))?
    };
    Ok(Registration {
        agent,
        session_id: sid(&session)?,
        cwd,
        urls: o.many("url"),
        item_ids: o.many("item"),
        id: o.optional("record"),
        title: o.optional("title"),
        project: o.optional("project"),
        destination: o.optional("destination"),
    })
}
fn review_payload(o: &Options, id: String) -> Result<Operation> {
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
        && (!o.get("file").is_empty() || !o.get("state").is_empty() || !o.get("pass").is_empty())
    {
        return Err(Error::new("Read the draft separately from review updates."));
    }
    if !o.has("show") && o.get("file").is_empty() && o.get("state").is_empty() {
        return Err(Error::new("Review needs --show, --file, or --state."));
    }
    if !o.get("state").is_empty() || !o.get("pass").is_empty() {
        sid(o.required("request")?)?;
    }
    let report = if o.get("file").is_empty() {
        None
    } else {
        Some(json_file(o, 96 * 1024)?)
    };
    let reviewed_revision = o
        .optional("reviewed-revision")
        .map(|revision| revision.parse::<u64>())
        .transpose()
        .map_err(|_| Error::new("Invalid review revision."))?;
    Ok(Operation::Review(ReviewUpdate {
        id,
        request_id: o.optional("request"),
        pass: o.optional("pass"),
        state: o.optional("state"),
        report,
        reviewed_revision,
    }))
}
fn context_payload(o: &Options, id: String) -> Result<Operation> {
    o.validate(&["record", "file", "show"])?;
    if o.has("show") != o.get("file").is_empty() {
        return Err(Error::new("Choose either --show or --file."));
    }
    if o.has("show") {
        return Ok(Operation::ContextRead(ContextRead { id }));
    }
    let context = json_file(o, 96 * 1024)?;
    require_context_revision(&context)?;
    Ok(Operation::ContextReplace(ContextReplace { id, context }))
}
fn resource_payload(o: &Options, id: String) -> Result<Operation> {
    o.validate(&["record", "file", "replace"])?;
    let mut report = json_file(o, 64 * 1024)?;
    if !report["resources"].is_array() {
        return Err(Error::new("Resource report needs a resources array."));
    }
    if o.has("replace") {
        require_context_revision(&report)?;
    }
    report["replace"] = json!(o.has("replace"));
    Ok(Operation::Resources(ResourceReport { id, report }))
}
fn worktree_payload(o: &Options, id: String) -> Result<Operation> {
    o.validate(&["record", "path", "url", "item", "primary"])?;
    let cwd = std::env::current_dir()?;
    Ok(Operation::Worktree(WorktreeReport {
        id,
        path: path(if o.get("path").is_empty() {
            cwd.to_str().ok_or_else(Error::io)?
        } else {
            o.get("path")
        })?,
        urls: o.many("url"),
        item_ids: o.many("item"),
        primary: o.has("primary"),
    }))
}
fn status_payload(o: &Options, id: String) -> Result<Operation> {
    o.validate(&["record", "state", "subagents", "reason"])?;
    let state = o.required("state")?.into();
    let subagents = o
        .optional("subagents")
        .map(|count| count.parse::<u64>())
        .transpose()
        .map_err(|_| Error::new("Subagent count must be nonnegative."))?;
    Ok(Operation::Runtime(StatusReport {
        id,
        state,
        subagents,
        reason: o.optional("reason"),
    }))
}
fn artifact_payload(o: &Options, id: String) -> Result<Operation> {
    o.validate(&["record", "file", "url", "title", "skill-output"])?;
    if o.get("file").is_empty() == o.get("url").is_empty() {
        return Err(Error::new("Choose a file or URL."));
    }
    Ok(Operation::SaveArtifact(ArtifactAttachment {
        id,
        path: o.optional("file").as_deref().map(path).transpose()?,
        url: o.optional("url").as_deref().map(safe_url).transpose()?,
        title: o.optional("title"),
        skill_output: o.has("skill-output"),
    }))
}
fn report_payload(command: &str, o: &Options, id: String) -> Result<Operation> {
    o.validate(&["record", "file"])?;
    let bytes = read(&expand(o.required("file")?), 96000)?;
    if command == "briefing" {
        let body = String::from_utf8(bytes).map_err(|_| Error::new("Report must be UTF-8."))?;
        return Ok(Operation::SaveBriefing(Briefing { id, body }));
    }
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
    Ok(Operation::Checkpoint(Checkpoint {
        id,
        done: object.get("done").cloned(),
        blocked: object.get("blocked").cloned(),
        next: object.get("next").cloned(),
    }))
}
fn runtime(root: &Path) -> Result<Value> {
    let value: Value = serde_json::from_slice(&read(&root.join("data/runtime.json"), 16384)?)?;
    let url = url::Url::parse(text(&value, "url"))
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
        .fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        });
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
        let client: ureq::Agent = ureq::Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .user_agent("")
            .timeout_global(Some(Duration::from_millis(if hook {
                1500
            } else {
                120_000
            })))
            .build()
            .into();
        match client
            .post(format!(
                "{}/api/agent/register",
                text(runtime, "url").trim_end_matches('/')
            ))
            .header(
                "Authorization",
                format!("Bearer {}", text(runtime, "token")),
            )
            .header("Content-Type", "application/json")
            .send(&serde_json::to_vec(payload)?)
        {
            Ok(response) => {
                let status = response.status();
                let mut bytes = Vec::new();
                response
                    .into_body()
                    .into_reader()
                    .take(1024 * 1024 + 1)
                    .read_to_end(&mut bytes)?;
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
            Err(error) if is_connect(&error) => {}
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
fn is_connect(error: &ureq::Error) -> bool {
    use std::io::ErrorKind;
    match error {
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => true,
        ureq::Error::Io(error) => matches!(
            error.kind(),
            ErrorKind::ConnectionRefused
                | ErrorKind::AddrNotAvailable
                | ErrorKind::NetworkUnreachable
                | ErrorKind::HostUnreachable
        ),
        _ => false,
    }
}
pub fn activity_snapshot(data: &Value) -> Background {
    let mut result = Background::default();
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
        result.background_counts = Some(counts);
    }
    if let Some(crons) = data["session_crons"].as_array() {
        result.scheduled_wakeups = Some(crons.len().min(1000) as u64);
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
    if let Some(payload) = hook_payload(o, &serde_json::from_slice(&bytes)?)? {
        send(&runtime(root)?, &payload, o.has("inbox"), true)?;
    }
    Ok(())
}
/// The runtime event for hook input `data`, or `None` for a subagent's own turn events.
fn hook_payload(o: &Options, data: &Value) -> Result<Option<Value>> {
    let event: Hook = serde_json::from_value(json!(o.required("event")?))
        .map_err(|_| Error::new("Unsupported runtime event."))?;
    if !text(data, "agent_id").is_empty()
        && matches!(
            event,
            Hook::SessionStart | Hook::Stop | Hook::UserPromptSubmit
        )
    {
        return Ok(None);
    }
    let event = RuntimeEvent {
        id: sid(o.required("record")?)?,
        event,
        session_id: text(data, "session_id").into(),
        subagent_id: if text(data, "agent_id").is_empty() {
            text(data, "subagent_id")
        } else {
            text(data, "agent_id")
        }
        .into(),
        background: if event.ends_turn() {
            activity_snapshot(data)
        } else {
            Background::default()
        },
    };
    Request::Operation(Operation::RuntimeEvent(event))
        .to_json()
        .map(Some)
}
fn shown_result<'a>(command: &str, result: &'a Value) -> &'a Value {
    &result[if command == "review" {
        "review"
    } else {
        "context"
    }]
}
fn command_help(command: &str) -> String {
    format!("{command}: --record UUID [--home DIRECTORY] [--file PATH] [--state STATE] [--show]\nregister: --agent claude|codex [--session UUID] [--cwd PATH] [--url URL] [--item ID]\nreview/ci: --request UUID [--pass NAME] [--reviewed-revision NUMBER]\ncommand: --record UUID --name COMMAND [--file ARGUMENTS_JSON] (writes show in that chat)\nsettings: --show OR --record UUID --file PATCH_JSON --revision NUMBER --content-hash HASH (new or changed command text needs the user; writes show in that chat)\ncontext: --record UUID --show OR --file CONTEXT_JSON (replacement requires its captured revision)\nresource: --record UUID --file REPORT_JSON [--replace]; --replace requires report.revision from context --show\nReplacement lists: omitted stays unchanged; [] clears dashboard links only, never external resources/files. On conflict re-read context and reapply intended edits; do not substitute a fresh revision into the stale file.\nUse --inbox to use the private signed local inbox.")
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
        println!("Truffle: {}\nUse COMMAND --help for options. Discover app commands with: command --record UUID --name commands.list\nService: --serve --home DIRECTORY", COMMANDS.join(", "));
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

#[cfg(test)]
mod wire_tests;
