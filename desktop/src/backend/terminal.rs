//! Terminal presentation port. Local CLI utilities and the optional Codex desktop opener.
use super::{directory, home, json, sid, text, Error, PathBuf, Result, Value};
use std::{
    io::{Read, Write},
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};
/// Waits up to `timeout` for `child` to exit. Returns `None` when it is still running.
pub fn wait_timeout(child: &mut Child, timeout: Duration) -> std::io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + timeout;
    let mut pause = Duration::from_millis(1);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(None);
        }
        std::thread::sleep(pause.min(left));
        pause = (pause * 2).min(Duration::from_millis(50));
    }
}
pub fn run(program: &str, args: &[String], input: Option<&str>, seconds: u64) -> Result<String> {
    let timeout = Duration::from_secs(seconds);
    match bounded(program, args, input, timeout)? {
        Output::TimedOut => Err(timed_out(program, timeout)),
        Output::Exited(status, stdout, _) if status.success() => {
            Ok(String::from_utf8_lossy(&stdout).into())
        }
        Output::Exited(_, _, stderr) => Err(failed(&stderr)),
    }
}
pub enum Output {
    /// Exit status, stdout and stderr.
    Exited(ExitStatus, Vec<u8>, Vec<u8>),
    TimedOut,
}
/// Runs `program` and kills it if it is still running after `timeout`.
pub fn bounded(
    program: &str,
    args: &[String],
    input: Option<&str>,
    timeout: Duration,
) -> Result<Output> {
    let mut child = Command::new(program)
        .args(args)
        // Callers parse stdout; gh colors JSON in a pipe when this is set.
        .env_remove("CLICOLOR_FORCE")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| {
            Error::new(format!(
                "Cannot start {program}. Check that it is installed."
            ))
        })?;
    // A child may fill stdout before it reads all of stdin, so the input has its own thread.
    let writer = input.map(|data| {
        let mut stdin = child.stdin.take().unwrap();
        let data = data.as_bytes().to_vec();
        std::thread::spawn(move || stdin.write_all(&data))
    });
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let output = std::thread::spawn(move || {
        let mut b = Vec::new();
        out.by_ref().take(16 * 1024 * 1024).read_to_end(&mut b).ok();
        b
    });
    let errors = std::thread::spawn(move || {
        let mut b = Vec::new();
        err.by_ref().take(1024 * 1024).read_to_end(&mut b).ok();
        b
    });
    let status = wait_timeout(&mut child, timeout)?;
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    let Some(status) = status else {
        return Ok(Output::TimedOut);
    };
    Ok(Output::Exited(
        status,
        output.join().unwrap_or_default(),
        errors.join().unwrap_or_default(),
    ))
}
pub fn timed_out(program: &str, timeout: Duration) -> Error {
    let mut e = Error::new(format!(
        "{program} timed out. Cached data remains available."
    ));
    e.retry = 300.0;
    e.data = json!({"kind":"timeout","transport":"local-cli","timeoutSeconds":timeout.as_secs()});
    e
}
pub fn failed(stderr: &[u8]) -> Error {
    let mut e = Error::new("Local command failed. Check account access or the selected command.");
    let line = String::from_utf8_lossy(stderr)
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(200)
        .collect::<String>();
    e.data = json!({"kind":"exit","transport":"local-cli","stderr":line});
    e
}
pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
pub fn executable(name: &str) -> String {
    for root in std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .chain(["/opt/homebrew/bin", "/usr/local/bin"])
    {
        let p = PathBuf::from(root).join(name);
        if p.is_file() {
            return p.to_string_lossy().into();
        }
    }
    let p = home().join(".local/bin").join(name);
    if p.is_file() {
        return p.to_string_lossy().into();
    }
    name.into()
}
#[derive(Debug, PartialEq, Eq)]
pub enum OpenOutcome {
    Accepted,
    Preview,
    Unknown,
}
fn desktop_command_outcome(mut command: Command, timeout: Duration) -> Result<OpenOutcome> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Error::new("The desktop opener could not start."))?;
    match wait_timeout(&mut child, timeout) {
        Ok(Some(status)) if status.success() => Ok(OpenOutcome::Accepted),
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            Ok(OpenOutcome::Unknown)
        }
    }
}
pub fn open_desktop(s: &Value, packet: &str) -> Result<OpenOutcome> {
    let id = text(s, "sessionId");
    let agent = text(s, "agent");
    let cwd = directory(text(s, "cwd"))?;
    if s["destination"] == "desktop" && agent == "codex" {
        let url = if id.is_empty() {
            let mut u = url::Url::parse("codex://new").unwrap();
            u.query_pairs_mut().append_pair("path", &cwd).append_pair(
                "prompt",
                &format!("Read the work context at {packet}. Follow the assigned instruction."),
            );
            u.to_string()
        } else {
            format!("codex://threads/{}", sid(id)?)
        };
        let mut command = Command::new("/usr/bin/open");
        command.arg(url);
        return desktop_command_outcome(command, Duration::from_secs(5));
    }
    Err(Error::new("Choose the embedded terminal or Codex desktop."))
}

#[cfg(test)]
mod open_outcome_tests {
    use super::*;
    #[test]
    fn saved_chat_desktop_open_classifies_only_prespawn_failures_as_rejected() {
        assert!(desktop_command_outcome(
            Command::new("/nonexistent/desktop-opener"),
            Duration::from_millis(50)
        )
        .is_err());
        assert_eq!(
            desktop_command_outcome(Command::new("/usr/bin/true"), Duration::from_secs(1)).unwrap(),
            OpenOutcome::Accepted
        );
        assert_eq!(
            desktop_command_outcome(Command::new("/usr/bin/false"), Duration::from_secs(1))
                .unwrap(),
            OpenOutcome::Unknown
        );
        let mut sleep = Command::new("/bin/sleep");
        sleep.arg("1");
        assert_eq!(
            desktop_command_outcome(sleep, Duration::from_millis(1)).unwrap(),
            OpenOutcome::Unknown
        );
    }
}

#[cfg(test)]
mod run_tests {
    use super::*;
    #[test]
    fn wait_timeout_returns_the_exit_status_or_none_while_running() {
        let mut quick = Command::new("/bin/sh")
            .args(["-c", "exit 3"])
            .spawn()
            .unwrap();
        let status = wait_timeout(&mut quick, Duration::from_secs(5)).unwrap();
        assert_eq!(status.and_then(|s| s.code()), Some(3));
        let mut slow = Command::new("/bin/sleep").arg("5").spawn().unwrap();
        let started = Instant::now();
        assert!(wait_timeout(&mut slow, Duration::from_millis(100))
            .unwrap()
            .is_none());
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert!(started.elapsed() < Duration::from_secs(2));
        slow.kill().unwrap();
        slow.wait().unwrap();
    }
    #[test]
    fn parsed_commands_never_inherit_forced_color() {
        let previous = std::env::var_os("CLICOLOR_FORCE");
        std::env::set_var("CLICOLOR_FORCE", "1");
        let output = run(
            "/bin/sh",
            &["-c".into(), "printf %s \"${CLICOLOR_FORCE-unset}\"".into()],
            None,
            5,
        );
        if let Some(value) = previous {
            std::env::set_var("CLICOLOR_FORCE", value);
        } else {
            std::env::remove_var("CLICOLOR_FORCE");
        }
        assert_eq!(output.unwrap(), "unset");
    }
    #[test]
    fn a_child_that_echoes_a_large_input_does_not_block() {
        let input = "x".repeat(1024 * 1024);
        let (sender, receiver) = std::sync::mpsc::channel();
        let expected = input.clone();
        std::thread::spawn(move || {
            let _ = sender.send(run("/bin/cat", &[], Some(&expected), 10));
        });
        let output = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("bounded blocked on a child that fills stdout before reading stdin");
        assert_eq!(output.unwrap(), input);
    }
}
