//! Backend-owned PTYs. Bounded output, exact record identity, no shell-command API.
use super::*;
use portable_pty::{native_pty_system, CommandBuilder, ChildKiller, MasterPty, PtySize};
use std::{
    collections::{BTreeMap, VecDeque},
    io::{Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    time::Duration,
};
const BUFFER_LIMIT: usize = 2 * 1024 * 1024;
/// Interactive children must not inherit a build runner's non-interactive color hints.
fn terminal_environment(command: &mut CommandBuilder) {
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
pub fn check_agent_storage(agent: &str) -> Result<()> {
    let name = match agent {
        "claude" => ".claude",
        "codex" => ".codex",
        _ => return Err(Error::new("Choose Claude or Codex.")),
    };
    let storage = if agent == "codex" {
        std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(name))
    } else {
        home().join(name)
    };
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
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    output: Mutex<Output>,
    changed: Condvar,
    alive: AtomicBool,
    exited: AtomicBool,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
}
#[derive(Default)]
pub struct EmbeddedHost {
    sessions: Mutex<BTreeMap<String, Arc<Session>>>,
}
impl EmbeddedHost {
    pub fn open_shell(&self,record:&Value)->Result<Value>{
        let cwd=directory(text(record,"cwd"))?;
        let mut command=CommandBuilder::new("/bin/zsh");command.arg("-l");command.cwd(cwd);terminal_environment(&mut command);
        self.spawn(record,command)?;Ok(json!({"id":record["id"]}))
    }
    pub fn open_demo(&self, record: &Value) -> Result<()> {
        let mut command = CommandBuilder::new("/bin/sh");
        command.arg("-i");
        command.cwd("/tmp");
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
            .ok_or(Error::new(
                "This terminal is no longer in memory. Resume the conversation from History.",
            ))
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
        let agent = text(record, "agent");
        check_agent_storage(agent)?;
        let mut command = CommandBuilder::new(super::terminal::executable(agent));
        command.cwd(&cwd);
        let prompt=format!("Read the private work context at {packet}. Follow the instruction and register this conversation with truffle before your first reply.");
        match (agent, new) {
            ("claude", true) => {
                command.args([
                    "--session-id",
                    &sid(text(record, "sessionId"))?,
                    "--name",
                    text(record, "title"),
                    &prompt,
                ]);
            }
            ("codex", true) => {
                command.args(["-C", &cwd, &prompt]);
            }
            ("claude", false) => {
                command.args(["--resume", &sid(text(record, "sessionId"))?]);
            }
            ("codex", false) => {
                command.args(["resume", &sid(text(record, "sessionId"))?, "-C", &cwd]);
            }
            _ => return Err(Error::new("Choose Claude or Codex.")),
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
            let callback=format!("{} hook --record {} --event {}",super::agent_cli::helper(root)?,text(record,"id"),event);
            hooks[event] = json!([{"hooks":[{"type":"command","command":callback,"timeout":3}]}]);
            if agent == "codex" {
                command.args([
                    "-c",
                    &format!(
                        "hooks.{event}=[{{hooks=[{{type=\"command\",command={},timeout=3}}]}}]",
                        json!(callback)
                    ),
                ]);
            }
        }
        if agent == "claude" {
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
        command.env("TRUFFLE_BIN", std::env::current_exe()?.to_string_lossy().as_ref());
        command.env("TRUFFLE_RECORD_ID", text(record, "id"));
        command.env("TRUFFLE_HOME", root.to_string_lossy().as_ref());
        self.spawn(record, command)?;
        Ok(
            json!({"message":"Agent terminal is ready in truffle.","embedded":true,"terminalId":record["id"]}),
        )
    }
    fn spawn(&self, record: &Value, command: CommandBuilder) -> Result<()> {
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
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 30,
                cols: 110,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|_| Error::new("Could not allocate a local terminal."))?;
        let mut reader = pair.master.try_clone_reader().map_err(|_| Error::io())?;
        let writer = pair.master.take_writer().map_err(|_| Error::io())?;
        let started_at = now();
        let mut child = pair.slave.spawn_command(command).map_err(|_| {
            Error::new("Could not start the agent. Check its CLI installation and working folder.")
        })?;
        drop(pair.slave);
        let session = Arc::new(Session {
            generation: uuid(),
            started_at,
            record: record.clone(),
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            output: Mutex::new(Output {
                bytes: VecDeque::new(),
                base: 0,
                end: 0,
            }),
            changed: Condvar::new(),
            alive: AtomicBool::new(true),
            exited: AtomicBool::new(false),
            killer: Mutex::new(child.clone_killer()),
        });
        sessions.insert(id, session.clone());
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
            return Err(Error::new("This terminal was replaced. Refresh before closing it."));
        }
        let mut killer = session.killer.lock().unwrap();
        if session.exited.load(Ordering::Relaxed) { return Ok(()); }
        // Closing a terminal sends HUP so the CLI can save its conversation.
        killer.kill().map_err(|_| Error::new("Could not close this agent terminal."))?;
        let output = session.output.lock().unwrap();
        let _wait = session.changed.wait_timeout_while(output, Duration::from_secs(2), |_| !session.exited.load(Ordering::Relaxed)).unwrap();
        if !session.exited.load(Ordering::Relaxed) {
            return Err(Error::new("The agent is still stopping. Try closing its tab again."));
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
            .skip((start - output.base) as usize)
            .take(32768)
            .copied()
            .collect::<Vec<_>>();
        Ok(
            json!({"generation":session.generation,"cursor":start+bytes.len()as u64,"hasMore":start+(bytes.len()as u64)<output.end,"bytes":bytes,"reset":reset,"truncated":reset&&output.base>0,"alive":session.alive.load(Ordering::Relaxed)}),
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
        Ok(())
    }
    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        if !(10..=500).contains(&cols) || !(2..=200).contains(&rows) {
            return Err(Error::new("Invalid terminal size."));
        }
        self.get(id)?
            .master
            .lock()
            .unwrap()
            .resize(PtySize {
                cols,
                rows,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|_| Error::io())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn close_stops_only_the_matching_terminal_generation_and_allows_resume() {
        let host = EmbeddedHost::default();
        let id = uuid(); let record = json!({"id":id});
        host.open_demo(&record).unwrap();
        let first = text(&host.inventory()[0], "generation").to_owned();
        assert!(host.close(&id, "wrong-generation").is_err());
        assert_eq!(host.inventory()[0]["live"], true);
        host.close(&id, &first).unwrap();
        assert_eq!(host.inventory()[0]["live"], false);
        host.close(&id, &first).unwrap();
        host.open_demo(&record).unwrap();
        let next = text(&host.inventory()[0], "generation").to_owned();
        assert_ne!(first, next);
        assert!(host.close(&id, &first).is_err());
        assert_eq!(host.inventory()[0]["live"], true);
        host.close(&id, &next).unwrap();
    }
    #[test]
    fn companion_shell_action_preserves_whitespace_input_and_validates_payloads() {
        let root = tempfile::tempdir().unwrap();
        let repo = Repository::open(root.path().into(), None, true).unwrap();
        let id = uuid();
        repo.transaction(|s| {
            s["chatTerminals"] = json!({"version":1,"byChat":{"fixture":{"id":id}}});
            Ok(())
        }).unwrap();
        // No interactive shell, login scripts, model, or persistent daemon.
        let mut command = CommandBuilder::new("/bin/cat");
        command.cwd(root.path());
        let super::super::terminal_daemon::Host::Local(host) = repo.shells.as_ref() else {
            panic!("Demo repository must use an isolated in-memory terminal host");
        };
        host.spawn(&json!({"id":id}), command).unwrap();
        let generation = text(&repo.shells.inventory()[0], "generation").to_owned();
        let outcome = (|| -> Result<()> {
            let input = |data: Value| super::super::chat_terminal::action(
                &repo, "/api/shell-terminal/input", &json!({"id":id,"data":data})
            );
            if super::super::chat_terminal::action(&repo, "/api/shell-terminal/input", &json!({"id":id})).is_ok()
                || input(json!(42)).is_ok()
                || input(json!("é".repeat(32769))).is_ok() {
                return Err(Error::new("Missing, numeric, or oversized terminal input was accepted."));
            }
            for packet in ["BEGIN", " ", "middle", "\t", "tail", "\r", "  pasted text  ", "\r"] {
                input(json!(packet))?;
            }
            let mut bytes = vec![];
            let mut cursor = 0;
            let expected = [b"BEGIN middle\ttail\r\n".as_slice(), b"  pasted text  \r\n".as_slice()];
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while std::time::Instant::now() < deadline {
                let packet = repo.shells.poll(&id, cursor, &generation)?;
                cursor = packet["cursor"].as_u64().unwrap();
                bytes.extend(arr(&packet["bytes"]).iter().map(|n| n.as_u64().unwrap() as u8));
                // The PTY echoes input too (expanding tabs); cat's returned line
                // retains the actual tab, distinguishing it from that echo.
                if expected.iter().all(|line| bytes.windows(line.len()).any(|window| window == *line)) { return Ok(()); }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(Error::new(format!("Companion input lost bytes: {:?}", String::from_utf8_lossy(&bytes))))
        })();
        // Close only this in-memory fixture's exact terminal generation even
        // if a validation or polling assertion below fails.
        repo.shells.close(&id, &generation).unwrap();
        outcome.unwrap();
    }

    #[test]
    fn interactive_environment_restores_color_without_changing_agent_security() {
        let mut command = CommandBuilder::new("/bin/sh");
        command.env("NO_COLOR", "1");
        command.env("CI", "1");
        command.env("CODEX_SANDBOX", "test-marker");
        command.env("HOME", "/preserved-home");
        terminal_environment(&mut command);
        assert!(command.get_env("NO_COLOR").is_none());
        assert!(command.get_env("CI").is_none());
        assert_eq!(command.get_env("FORCE_COLOR").unwrap(), "3");
        assert_eq!(command.get_env("TERM").unwrap(), "xterm-256color");
        assert_eq!(command.get_env("HOME").unwrap(), "/preserved-home");
        assert_eq!(command.get_env("CODEX_SANDBOX").unwrap(), "test-marker");
    }
    #[test]
    fn storage_probe_leaves_no_files_and_rejects_unavailable_paths() {
        let temp = tempfile::tempdir().unwrap();
        probe_storage(temp.path()).unwrap();
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
        assert!(probe_storage(&temp.path().join("missing")).is_err());
    }
    #[test]
    fn exited_pty_replays_all_buffered_chunks() {
        let host = EmbeddedHost::default();
        let id = uuid();
        let record = json!({"id":id});
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", "head -c 100000 /dev/zero; printf END"]);
        host.spawn(&record, command).unwrap();
        for _ in 0..100 {
            if host.inventory()[0]["live"] == false {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(host.inventory()[0]["live"], false);
        let mut cursor = 0;
        let mut generation = String::new();
        let mut bytes = vec![];
        loop {
            let p = host.poll(&id, cursor, &generation).unwrap();
            cursor = p["cursor"].as_u64().unwrap();
            generation = text(&p, "generation").into();
            bytes.extend(arr(&p["bytes"]).iter().map(|x| x.as_u64().unwrap() as u8));
            if p["alive"] == false && p["hasMore"] == false {
                break;
            }
        }
        assert_eq!(bytes.len(), 100003);
        assert!(bytes.ends_with(b"END"));
    }
    #[test]
    fn pty_streams_input_and_resize_and_reaps_exit() {
        let host = EmbeddedHost::default();
        let id = uuid();
        let record =
            json!({"id":id,"agent":"claude","title":"PTY test","sessionId":uuid(),"cwd":"/tmp"});
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", "read line; stty size; printf 'PTY:%s\\n' \"$line\""]);
        host.spawn(&record, command).unwrap();
        host.resize(&id, 93, 27).unwrap();
        host.input(&id, "hello\n").unwrap();
        let mut cursor = 0;
        let mut generation = String::new();
        let mut result = vec![];
        for _ in 0..10 {
            let poll = host.poll(&id, cursor, &generation).unwrap();
            cursor = poll["cursor"].as_u64().unwrap();
            generation = text(&poll, "generation").into();
            result.extend(
                arr(&poll["bytes"])
                    .iter()
                    .map(|x| x.as_u64().unwrap() as u8),
            );
            if poll["alive"] == false {
                break;
            }
        }
        let output = String::from_utf8_lossy(&result);
        assert!(output.contains("27 93"), "{output}");
        assert!(output.contains("PTY:hello"), "{output}");
        for _ in 0..20 {
            if host.inventory()[0]["live"] == false {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(host.inventory()[0]["live"], false);
        assert!(host.input(&id, "again").is_err());
    }
}
