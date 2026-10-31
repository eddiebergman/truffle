//! Backend-owned PTYs. Bounded output, exact record identity, no shell-command API.
use super::agents::Agent;
use super::pty::{Size, Terminal};
use super::{directory, home, json, now, sid, text, uuid, Error, PathBuf, Result, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    io::{Read, Write},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    time::Duration,
};
const BUFFER_LIMIT: usize = 2 * 1024 * 1024;
/// Interactive children must not inherit a build runner's non-interactive color hints.
fn terminal_environment(command: &mut Command) {
    for key in [
        "NO_COLOR",
        "CLICOLOR",
        "CLICOLOR_FORCE",
        "FORCE_COLOR",
        "CI",
    ] {
        command.env_remove(key);
    }
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("TERM_PROGRAM", "truffle");
    command.env("FORCE_COLOR", "3");
    command.env("CLICOLOR", "1");
    command.env("CLICOLOR_FORCE", "1");
}
/// Check the service's actual filesystem access before an agent can lose transcripts.
/// This does not change permissions, sandbox configuration, HOME, or agent settings.
pub fn check_agent_storage(agent: Agent) -> Result<()> {
    let storage = agent.home();
    let directory = if storage.is_dir() {
        storage.clone()
    } else {
        home()
    };
    probe_storage(&directory).map_err(|_|Error::new(format!("The truffle service cannot write {storage}. Start ~/code/mydash/start.command from a regular Terminal window, outside a coding-agent sandbox, before launching an embedded agent. Agent permission settings were not changed.", storage=storage.display())))
}
fn probe_storage(directory: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = directory.join(format!(".truffle-write-check-{}", uuid()));
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    drop(file);
    std::fs::remove_file(path)
}

struct Output {
    bytes: VecDeque<u8>,
    base: u64,
    end: u64,
}
pub struct Session {
    generation: String,
    started_at: f64,
    record: Value,
    terminal: Terminal,
    writer: Mutex<super::pty::Writer>,
    output: Mutex<Output>,
    changed: Condvar,
    alive: AtomicBool,
    exited: AtomicBool,
    /// Held while closing, so only one close signals the child.
    pid: Mutex<u32>,
}
#[derive(Default)]
pub struct EmbeddedHost {
    sessions: Mutex<BTreeMap<String, Arc<Session>>>,
}
impl EmbeddedHost {
    pub fn open_shell(&self, record: &Value) -> Result<Value> {
        let cwd = directory(text(record, "cwd"))?;
        let mut command = Command::new("/bin/zsh");
        command.arg("-l");
        command.current_dir(cwd);
        terminal_environment(&mut command);
        self.spawn(record, command)?;
        Ok(json!({"id":record["id"]}))
    }
    pub fn open_demo(&self, record: &Value) -> Result<()> {
        let mut command = Command::new("/bin/sh");
        command.arg("-i");
        command.current_dir("/tmp");
        terminal_environment(&mut command);
        command.env("PS1", "truffle smoke $ ");
        self.spawn(record, command)
    }
    fn get(&self, id: &str) -> Result<Arc<Session>> {
        self.sessions
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| {
                Error::new(
                    "This terminal is no longer in memory. Resume the conversation from History.",
                )
            })
    }
    pub fn inventory(&self) -> Vec<Value> {
        self.sessions.lock().unwrap().iter().map(|(id,s)|json!({"host":"embedded","recordId":id,"sessionId":s.record["sessionId"],"agent":s.record["agent"],"title":s.record["title"],"cwd":s.record["cwd"],"live":s.alive.load(Ordering::Relaxed),"running":s.alive.load(Ordering::Relaxed),"active":s.alive.load(Ordering::Relaxed),"lifecycle":if s.alive.load(Ordering::Relaxed){"unknown"}else{"closed"},"generation":s.generation,"startedAt":s.started_at})).collect()
    }
    pub fn open(
        &self,
        record: &Value,
        packet: &str,
        new: bool,
        root: &std::path::Path,
    ) -> Result<Value> {
        let cwd = directory(text(record, "cwd"))?;
        let agent = Agent::parse(text(record, "agent"))?;
        check_agent_storage(agent)?;
        let mut command = Command::new(super::terminal::executable(agent.name()));
        command.current_dir(&cwd);
        let prompt=format!("Read the private work context at {packet}. Follow the instruction and register this conversation with truffle before your first reply.");
        match (agent, new) {
            (Agent::Claude, true) => {
                command.args([
                    "--session-id",
                    &sid(text(record, "sessionId"))?,
                    "--name",
                    text(record, "title"),
                    &prompt,
                ]);
            }
            (Agent::Codex, true) => {
                command.args(["-C", &cwd, &prompt]);
            }
            (Agent::Claude, false) => {
                command.args(["--resume", &sid(text(record, "sessionId"))?]);
            }
            (Agent::Codex, false) => {
                command.args(["resume", &sid(text(record, "sessionId"))?, "-C", &cwd]);
            }
        }
        let mut hooks = json!({});
        for event in [
            "SessionStart",
            "UserPromptSubmit",
            "Stop",
            "PermissionRequest",
            "SubagentStart",
            "SubagentStop",
        ] {
            let callback = format!(
                "{} hook --record {} --event {}",
                super::agent_cli::helper(root)?,
                text(record, "id"),
                event
            );
            hooks[event] = json!([{"hooks":[{"type":"command","command":callback,"timeout":3}]}]);
            if agent == Agent::Codex {
                command.args([
                    "-c",
                    &format!(
                        "hooks.{event}=[{{hooks=[{{type=\"command\",command={},timeout=3}}]}}]",
                        json!(callback)
                    ),
                ]);
            }
        }
        if agent == Agent::Claude {
            let path = PathBuf::from(packet).with_extension("hooks.json");
            super::http::private_write(&path, &serde_json::to_vec(&json!({"hooks":hooks}))?)?;
            command.args(["--settings", &path.to_string_lossy()]);
        }
        for (key, _) in std::env::vars() {
            if key.starts_with("CMUX_")
                || ["CLAUDECODE", "CLAUDE_CODE_SESSION_ID", "CODEX_THREAD_ID"]
                    .contains(&key.as_str())
            {
                command.env_remove(key);
            }
        }
        terminal_environment(&mut command);
        command.env(
            "TRUFFLE_BIN",
            std::env::current_exe()?.to_string_lossy().as_ref(),
        );
        command.env("TRUFFLE_RECORD_ID", text(record, "id"));
        command.env("TRUFFLE_HOME", root.to_string_lossy().as_ref());
        self.spawn(record, command)?;
        Ok(
            json!({"message":"Agent terminal is ready in truffle.","embedded":true,"terminalId":record["id"]}),
        )
    }
    fn spawn(&self, record: &Value, command: Command) -> Result<()> {
        let mut sessions = self.sessions.lock().unwrap();
        let id = text(record, "id").to_owned();
        if sessions
            .get(&id)
            .is_some_and(|s| s.alive.load(Ordering::Relaxed))
        {
            return Ok(());
        }
        if sessions
            .values()
            .filter(|s| s.alive.load(Ordering::Relaxed))
            .count()
            >= 12
        {
            return Err(Error::new(
                "Up to 12 embedded agent terminals can run at once.",
            ));
        }
        if sessions.len() >= 32 {
            sessions.retain(|_, s| s.alive.load(Ordering::Relaxed));
        }
        let started_at = now();
        let (terminal, mut child) = Terminal::spawn(
            command,
            Size {
                cols: 110,
                rows: 30,
            },
        )
        .map_err(|_| {
            Error::new("Could not start the agent. Check its CLI installation and working folder.")
        })?;
        let mut reader = terminal.reader().map_err(|_| Error::io())?;
        let writer = terminal.writer().map_err(|_| Error::io())?;
        let session = Arc::new(Session {
            generation: uuid(),
            started_at,
            record: record.clone(),
            pid: Mutex::new(child.id()),
            terminal,
            writer: Mutex::new(writer),
            output: Mutex::new(Output {
                bytes: VecDeque::new(),
                base: 0,
                end: 0,
            }),
            changed: Condvar::new(),
            alive: AtomicBool::new(true),
            exited: AtomicBool::new(false),
        });
        sessions.insert(id, session.clone());
        drop(sessions);
        let output = session.clone();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 16384];
            while let Ok(count) = reader.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                let mut data = output.output.lock().unwrap();
                data.bytes.extend(&buffer[..count]);
                data.end += count as u64;
                while data.bytes.len() > BUFFER_LIMIT {
                    data.bytes.pop_front();
                    data.base += 1;
                }
                drop(data);
                output.changed.notify_all();
            }
            output.alive.store(false, Ordering::Relaxed);
            output.changed.notify_all();
        });
        std::thread::spawn(move || {
            let _ = child.wait();
            let _guard = session.output.lock().unwrap();
            session.exited.store(true, Ordering::Relaxed);
            session.changed.notify_all();
        });
        Ok(())
    }
    pub fn close(&self, id: &str, generation: &str) -> Result<()> {
        let session = self.get(id)?;
        if generation != session.generation {
            return Err(Error::new(
                "This terminal was replaced. Refresh before closing it.",
            ));
        }
        let pid = session.pid.lock().unwrap();
        if session.exited.load(Ordering::Relaxed) {
            return Ok(());
        }
        // Closing a terminal sends HUP so the CLI can save its conversation.
        super::pty::hang_up(*pid)
            .map_err(|_| Error::new("Could not close this agent terminal."))?;
        drop(pid);
        drop(
            session
                .changed
                .wait_timeout_while(
                    session.output.lock().unwrap(),
                    Duration::from_secs(2),
                    |_| !session.exited.load(Ordering::Relaxed),
                )
                .unwrap(),
        );
        if !session.exited.load(Ordering::Relaxed) {
            return Err(Error::new(
                "The agent is still stopping. Try closing its tab again.",
            ));
        }
        session.alive.store(false, Ordering::Relaxed);
        session.changed.notify_all();
        Ok(())
    }
    pub fn poll(&self, id: &str, cursor: u64, generation: &str) -> Result<Value> {
        let session = self.get(id)?;
        let mut output = session.output.lock().unwrap();
        if cursor == output.end
            && generation == session.generation
            && session.alive.load(Ordering::Relaxed)
        {
            output = session
                .changed
                .wait_timeout(output, Duration::from_millis(900))
                .unwrap()
                .0;
        }
        let reset = generation != session.generation || cursor < output.base || cursor > output.end;
        let start = if reset { output.base } else { cursor };
        let bytes = output
            .bytes
            .iter()
            .skip(usize::try_from(start - output.base).map_err(|_| Error::io())?)
            .take(32768)
            .copied()
            .collect::<Vec<_>>();
        let has_more = start + (bytes.len() as u64) < output.end;
        let truncated = reset && output.base > 0;
        let alive = session.alive.load(Ordering::Relaxed);
        drop(output);
        Ok(
            json!({"generation":session.generation,"cursor":start+bytes.len()as u64,"hasMore":has_more,"bytes":bytes,"reset":reset,"truncated":truncated,"alive":alive}),
        )
    }
    pub fn input(&self, id: &str, data: &str) -> Result<()> {
        if data.len() > 65536 {
            return Err(Error::new("Paste up to 64 KiB at a time."));
        }
        let session = self.get(id)?;
        if !session.alive.load(Ordering::Relaxed) {
            return Err(Error::new("This agent has exited. Resume it from History."));
        }
        let mut writer = session.writer.lock().unwrap();
        writer.write_all(data.as_bytes())?;
        writer.flush()?;
        drop(writer);
        Ok(())
    }
    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        if !(10..=500).contains(&cols) || !(2..=200).contains(&rows) {
            return Err(Error::new("Invalid terminal size."));
        }
        self.get(id)?
            .terminal
            .resize(Size { cols, rows })
            .map_err(|_| Error::io())
    }
}
#[cfg(test)]
mod tests;
