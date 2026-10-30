//! One non-interactive agent run. The agent loads no user customization, gets a cleared
//! environment and its own session, and its whole process group dies at the timeout or on cancel.
use super::agents::Agent;
use super::{json, text, uuid, Error, PathBuf, Result, Value};
use std::{
    io::{Read, Write},
    os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt},
        process::CommandExt,
    },
    path::Path,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

/// Only its size reaches the diagnostics.
const STDERR_LIMIT: usize = 128 * 1024;
const CODEX_DISABLED: &[&str] = &[
    "goals",
    "collaboration_modes",
    "default_mode_request_user_input",
    "hooks",
    "memories",
    "skill_search",
    "shell_snapshot",
    "request_permissions_tool",
    "shell_tool",
    "apps",
    "plugins",
    "multi_agent",
    "browser_use",
    "computer_use",
    "view_image",
    "code_mode",
    "code_mode_host",
    "enable_request_compression",
];

pub enum Mode<'a> {
    /// A new conversation that is never saved. It runs in an empty private folder.
    Ask,
    /// An unsaved copy of conversation `session`, run in `cwd`. The original is unchanged.
    ForkSession { session: &'a str, cwd: &'a Path },
}
pub struct Request<'a> {
    /// Sent on stdin, so it never reaches the command line.
    pub prompt: &'a str,
    /// Claude's system prompt or Codex's developer instructions.
    pub instructions: &'a str,
    pub mode: Mode<'a>,
    /// JSON Schema of the final answer.
    pub schema: Option<&'a Value>,
    /// Checked base64 image blocks.
    pub images: &'a [Value],
    /// Parent folder of this run's private files, which are removed afterwards.
    pub scratch: &'a Path,
    /// Stdout of this size or more fails the run.
    pub limit: usize,
    pub timeout: Duration,
    pub cancel: &'a AtomicBool,
}
#[derive(Debug)]
pub enum Failure {
    /// The run could not be prepared; the agent never started.
    Setup(Error),
    Spawn,
    Wait,
    Cancelled,
    TimedOut,
    Exited,
    Oversized,
    InvalidText,
}
impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self::Setup(error)
    }
}
impl From<std::io::Error> for Failure {
    fn from(error: std::io::Error) -> Self {
        Self::Setup(error.into())
    }
}

/// Runs `agent` once and returns its stdout. `diagnostics` gets sizes, timing and a failure
/// code, never output contents.
pub fn run(
    agent: Agent,
    request: &Request,
    diagnostics: &mut Value,
) -> std::result::Result<String, Failure> {
    run_program(
        &super::terminal::executable(agent.name()),
        agent,
        request,
        diagnostics,
    )
}
fn run_program(
    program: &str,
    agent: Agent,
    request: &Request,
    diagnostics: &mut Value,
) -> std::result::Result<String, Failure> {
    let files = Files::new(request.scratch)?;
    let command = build(program, agent, request, &files, std::env::vars())?;
    execute(command, &input(agent, request), request, diagnostics)
}

fn build(
    program: &str,
    agent: Agent,
    request: &Request,
    files: &Files,
    environment: impl IntoIterator<Item = (String, String)>,
) -> Result<Command> {
    let mut command = Command::new(program);
    match agent {
        Agent::Claude => claude_arguments(&mut command, request, files),
        Agent::Codex => codex_arguments(&mut command, request, files)?,
    }
    command
        .env_clear()
        .envs(allowed_environment(agent, environment));
    Ok(command)
}
fn claude_arguments(command: &mut Command, request: &Request, files: &Files) {
    command
        .args([
            "--safe-mode",
            "--print",
            "--setting-sources",
            "",
            "--no-session-persistence",
            "--tools",
            "",
            "--strict-mcp-config",
            "--mcp-config",
            "{\"mcpServers\":{}}",
            "--settings",
            "{\"disableAllHooks\":true}",
            "--disable-slash-commands",
            "--max-budget-usd",
            "3",
            "--system-prompt",
            request.instructions,
        ])
        .args(if request.images.is_empty() {
            ["--output-format", "json"].as_slice()
        } else {
            [
                "--output-format",
                "stream-json",
                "--input-format",
                "stream-json",
                "--verbose",
            ]
            .as_slice()
        });
    if let Some(schema) = request.schema {
        command.args(["--json-schema", &schema.to_string()]);
    }
    match request.mode {
        Mode::Ask => {
            command.current_dir(&files.root);
        }
        Mode::ForkSession { session, cwd } => {
            // The copy must use this system prompt, not the one recorded with the original.
            command
                .args([
                    "--resume",
                    session,
                    "--fork-session",
                    "--system-prompt-snapshot",
                    "off",
                ])
                .current_dir(cwd);
        }
    }
}
fn codex_arguments(command: &mut Command, request: &Request, files: &Files) -> Result<()> {
    command.args([
        "--no-daemon",
        "exec",
        "--ignore-user-config",
        "--ignore-rules",
        "--ephemeral",
        "--skip-git-repo-check",
        "--sandbox",
        "read-only",
        "--json",
        "-c",
        "web_search=\"disabled\"",
    ]);
    if let Some(schema) = request.schema {
        let path = files.write("response-schema.json", schema.to_string().as_bytes())?;
        command.arg("--output-schema").arg(path);
    }
    command.arg("-c").arg(format!(
        "developer_instructions={}",
        serde_json::to_string(request.instructions)?
    ));
    for feature in CODEX_DISABLED {
        command.args(["--disable", feature]);
    }
    command.args([
        "-c",
        "skills.include_instructions=false",
        "-c",
        "skills.bundled.enabled=false",
        "-c",
        "project_doc_max_bytes=0",
    ]);
    let images = files.images(request.images)?;
    if !images.is_empty() {
        if images
            .iter()
            .any(|path| path.to_string_lossy().contains(','))
        {
            return Err(Error::new("The screenshot runtime path contains a comma. Choose a local app data folder without commas."));
        }
        command.arg("--image").arg(
            images
                .iter()
                .map(|path| path.to_string_lossy())
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    let cwd = match request.mode {
        Mode::Ask => &files.root,
        Mode::ForkSession { cwd, .. } => cwd,
    };
    command.arg("-C").arg(cwd).current_dir(cwd);
    if let Mode::ForkSession { session, .. } = request.mode {
        command.args(["fork", session]);
    }
    command.arg("-");
    Ok(())
}
/// What the CLIs need to find their configuration and sign-in. Everything else, including
/// truffle's own agent token and record ID, is left out.
fn allowed_environment(
    agent: Agent,
    environment: impl IntoIterator<Item = (String, String)>,
) -> Vec<(String, String)> {
    let own: &[&str] = match agent {
        Agent::Claude => &[
            "CLAUDE_CONFIG_DIR",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_BASE_URL",
        ],
        Agent::Codex => &["CODEX_HOME", "CODEX_API_KEY"],
    };
    environment
        .into_iter()
        .filter(|(key, _)| {
            [
                "HOME", "PATH", "USER", "LOGNAME", "SHELL", "TMPDIR", "LANG", "LC_ALL", "TERM",
            ]
            .contains(&key.as_str())
                || own.contains(&key.as_str())
        })
        .collect()
}
fn input(agent: Agent, request: &Request) -> String {
    if agent == Agent::Codex || request.images.is_empty() {
        return request.prompt.to_owned();
    }
    let mut content = vec![json!({"type":"text","text":request.prompt})];
    content.extend_from_slice(request.images);
    format!(
        "{}\n",
        json!({"type":"user","message":{"role":"user","content":content},"parent_tool_use_id":null})
    )
}

/// Run-owned files never enter persistent app metadata or the conversation.
struct Files {
    root: PathBuf,
}
impl Files {
    fn new(parent: &Path) -> Result<Self> {
        std::fs::create_dir_all(parent)?;
        std::fs::set_permissions(parent, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
        let root = parent.join(format!("reply-{}", uuid()));
        std::fs::DirBuilder::new().mode(0o700).create(&root)?;
        Ok(Self { root })
    }
    fn write(&self, name: &str, bytes: &[u8]) -> Result<PathBuf> {
        let path = self.root.join(name);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(bytes)?;
        Ok(path)
    }
    fn images(&self, images: &[Value]) -> Result<Vec<PathBuf>> {
        use base64::Engine;
        images
            .iter()
            .enumerate()
            .map(|(index, image)| {
                let source = &image["source"];
                let extension = match text(source, "media_type") {
                    "image/png" => "png",
                    "image/jpeg" => "jpg",
                    _ => return Err(Error::new("Invalid screenshot format.")),
                };
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(text(source, "data"))
                    .map_err(|_| Error::new("Invalid screenshot data."))?;
                self.write(&format!("image-{index}.{extension}"), &bytes)
            })
            .collect()
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The kind of text in `value`, without its contents.
pub fn shape(value: &str) -> &'static str {
    if value.trim().is_empty() {
        return "empty";
    }
    match serde_json::from_str::<Value>(value) {
        Ok(Value::Object(_)) => "object",
        Ok(Value::Array(_)) => "array",
        Ok(Value::String(_)) => "string",
        Ok(Value::Number(_)) => "number",
        Ok(Value::Bool(_)) => "boolean",
        Ok(Value::Null) => "null",
        Err(_)
            if value.lines().filter(|line| !line.trim().is_empty()).count() > 1
                && value
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .all(|line| serde_json::from_str::<Value>(line).is_ok()) =>
        {
            "json-lines"
        }
        Err(_) => "non-json",
    }
}
fn capture(
    mut stream: impl Read + Send + 'static,
    limit: usize,
) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut saved = Vec::new();
        let mut buf = [0; 4096];
        while let Ok(n) = stream.read(&mut buf) {
            if n == 0 {
                break;
            }
            if saved.len() < limit {
                saved.extend_from_slice(&buf[..n.min(limit - saved.len())]);
            }
        }
        saved
    })
}
fn kill_group(child: &std::process::Child) {
    if let Ok(pid) = libc::pid_t::try_from(child.id()) {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
}
fn execute(
    mut command: Command,
    input: &str,
    request: &Request,
    diagnostics: &mut Value,
) -> std::result::Result<String, Failure> {
    command
        .stdin(Stdio::piped())
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
    diagnostics["stage"] = json!("launch");
    let mut child = command.spawn().map_err(|_| {
        diagnostics["code"] = json!("spawn_failed");
        Failure::Spawn
    })?;
    let mut stdin = child.stdin.take().unwrap();
    let input = input.as_bytes().to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let readers = [
        capture(child.stdout.take().unwrap(), request.limit),
        capture(child.stderr.take().unwrap(), STDERR_LIMIT),
    ];
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None)
                if start.elapsed() < request.timeout && !request.cancel.load(Ordering::Relaxed) =>
            {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => {
                kill_group(&child);
                let _ = child.wait();
                break Err(("process_wait_failed", Failure::Wait));
            }
            _ => {
                kill_group(&child);
                let _ = child.wait();
                break Err(if request.cancel.load(Ordering::Relaxed) {
                    ("cancelled", Failure::Cancelled)
                } else {
                    ("timed_out", Failure::TimedOut)
                });
            }
        }
    };
    kill_group(&child);
    let _ = writer.join();
    let mut outputs = readers.into_iter().map(|r| r.join().unwrap_or_default());
    let stdout = outputs.next().unwrap_or_default();
    let stderr = outputs.next().unwrap_or_default();
    diagnostics["stage"] = json!("process");
    diagnostics["durationMs"] =
        json!(u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX));
    diagnostics["stdoutBytes"] = json!(stdout.len());
    diagnostics["stderrBytes"] = json!(stderr.len());
    diagnostics["stdoutShape"] = json!(shape(&String::from_utf8_lossy(&stdout)));
    let status = status.map_err(|(code, failure)| {
        diagnostics["code"] = json!(code);
        failure
    })?;
    diagnostics["exitCode"] = json!(status.code());
    if !status.success() {
        diagnostics["code"] = json!("process_failed");
        return Err(Failure::Exited);
    }
    if stdout.len() >= request.limit {
        diagnostics["code"] = json!("oversized_output");
        return Err(Failure::Oversized);
    }
    String::from_utf8(stdout).map_err(|_| {
        diagnostics["code"] = json!("invalid_utf8");
        Failure::InvalidText
    })
}

#[cfg(test)]
mod tests;
