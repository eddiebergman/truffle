//! Real helper/process restart regression, using an isolated HOME and a fake agent.
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
const BIN: &str = env!("CARGO_BIN_EXE_truffle-desktop");
fn read(p: impl AsRef<Path>) -> Value {
    serde_json::from_slice(&fs::read(p).unwrap()).unwrap()
}
fn write(p: impl AsRef<Path>, v: &Value) {
    fs::write(p, serde_json::to_vec(v).unwrap()).unwrap();
}
fn wait(mut predicate: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(40);
    while Instant::now() < until {
        if predicate() {
            return;
        }
        std::thread::sleep(Duration::from_millis(80));
    }
    panic!("Fixture timed out");
}
struct Fixture {
    root: tempfile::TempDir,
    socket: PathBuf,
    url: String,
}
impl Fixture {
    fn command(&self) -> Command {
        let mut c = Command::new(BIN);
        c.env("HOME", self.root.path())
            .env("PATH", self.root.path().join("bin"))
            .env("SENTRY_ORG", "")
            .env("SENTRY_AUTH_TOKEN", "");
        c
    }
    fn control(&self, action: &str, extra: &[&str]) -> std::process::Output {
        self.command()
            .args(["--restart-control", action, "--home"])
            .arg(self.root.path())
            .args(["--url", &self.url])
            .args(extra)
            .output()
            .unwrap()
    }
    fn preview(&self) -> Value {
        let out = self.control("preview", &[]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }
    fn rpc(&self, p: Value) -> Value {
        let mut s = UnixStream::connect(self.socket.join("terminal.sock")).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
        let bytes = serde_json::to_vec(&json!({"protocol":1,"request":p})).unwrap();
        s.write_all(&(bytes.len() as u32).to_be_bytes()).unwrap();
        s.write_all(&bytes).unwrap();
        let mut n = [0; 4];
        s.read_exact(&mut n).unwrap();
        let size = u32::from_be_bytes(n) as usize;
        assert!(size < 512 * 1024);
        let mut out = vec![0; size];
        s.read_exact(&mut out).unwrap();
        let reply: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(reply["ok"], true, "{reply}");
        reply["value"].clone()
    }
    fn restart(&self, mode: &str) -> Value {
        let current = self.root.path().join("data/restarts/current.json");
        if current.exists() {
            let mut job = read(&current);
            job["finishedAt"] = json!(0);
            write(&current, &job);
        }
        let review = self.preview();
        let snapshot = review["snapshot"].as_str().unwrap();
        let out = self.control(
            "start",
            &[
                "--mode",
                mode,
                "--snapshot",
                snapshot,
                "--confirm-interruption",
            ],
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        wait(|| read(&current)["status"] != "working");
        let job = read(current);
        assert_eq!(job["status"], "succeeded", "{job}");
        job
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // These PIDs come only from this temporary profile, never the user's dashboard.
        let runtime = self.root.path().join("data/runtime.json");
        if runtime.exists() {
            if let Some(pid) = read(runtime)["pid"].as_i64() {
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
            }
        }
        if self.socket.join("terminal.sock").exists() {
            if let Ok(mut s) = UnixStream::connect(self.socket.join("terminal.sock")) {
                let bytes =
                    serde_json::to_vec(&json!({"protocol":1,"request":{"op":"health"}})).unwrap();
                let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = s.write_all(&(bytes.len() as u32).to_be_bytes());
                let _ = s.write_all(&bytes);
                let mut n = [0; 4];
                if s.read_exact(&mut n).is_ok() {
                    let size = u32::from_be_bytes(n) as usize;
                    if size < 65536 {
                        let mut out = vec![0; size];
                        if s.read_exact(&mut out).is_ok() {
                            if let Ok(v) = serde_json::from_slice::<Value>(&out) {
                                if let Some(pid) = v["value"]["pid"].as_i64() {
                                    unsafe {
                                        libc::kill(pid as i32, libc::SIGTERM);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn restart_migrates_legacy_preserves_live_shell_and_resumes_terminal_host() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().canonicalize().unwrap();
    fs::create_dir_all(path.join("data")).unwrap();
    fs::create_dir(path.join("bin")).unwrap();
    fs::create_dir(path.join(".claude")).unwrap();
    let cwd = path.join("worktree with spaces");
    fs::create_dir(&cwd).unwrap();
    let fake = path.join("bin/claude");
    fs::write(&fake,"#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$HOME/agent-args\"\npwd > \"$HOME/agent-cwd\"\nexec /bin/sh -i\n").unwrap();
    fs::set_permissions(fake, fs::Permissions::from_mode(0o700)).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let sid = uuid::Uuid::new_v4().to_string();
    let data = path.join("data/state.json");
    write(
        &data,
        &json!({"version":1,"sessions":[{"id":id,"sessionId":sid,"agent":"claude","title":"Restart fixture","cwd":cwd,"destination":"embedded","items":[]}],"sources":{"github":{"disabled":true},"linear":{"disabled":true},"sentry":{"disabled":true}}}),
    );
    let hash = context_kit::sha256(data.to_string_lossy().as_bytes());
    let socket = PathBuf::from(format!(
        "/tmp/truffle-pty-{}-{}",
        unsafe { libc::geteuid() },
        &hash[..16]
    ));
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let f = Fixture {
        root,
        socket,
        url: format!("http://127.0.0.1:{port}"),
    };
    let mut old = f
        .command()
        .args(["--serve", "--demo", "--home"])
        .arg(&path)
        .arg("--data")
        .arg(&data)
        .args(["--port", &port.to_string(), "--terminal-demo", &id])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    wait(|| path.join("data/demo-runtime.json").exists());
    fs::copy(
        path.join("data/demo-runtime.json"),
        path.join("data/runtime.json"),
    )
    .unwrap();
    let initial = f.preview();
    assert_eq!(initial["liveCount"], 1);
    assert_eq!(initial["preserveTerminals"], false);
    let denied = f.control(
        "start",
        &["--snapshot", initial["snapshot"].as_str().unwrap()],
    );
    assert!(!denied.status.success());
    assert!(old.try_wait().unwrap().is_none());
    let stale = f.control(
        "start",
        &["--snapshot", "outdated", "--confirm-interruption"],
    );
    assert!(!stale.status.success());
    assert!(old.try_wait().unwrap().is_none());
    f.restart("service");
    let _ = old.wait();
    wait(|| path.join("agent-args").exists());
    assert!(fs::read_to_string(path.join("agent-args"))
        .unwrap()
        .contains(&format!("--resume\n{sid}\n")));
    assert_eq!(
        fs::read_to_string(path.join("agent-cwd")).unwrap().trim(),
        cwd.to_str().unwrap()
    );
    let before = f.preview();
    assert_eq!(before["preserveTerminals"], true);
    let generation = before["sessions"][0]["generation"].clone();
    let daemon = before["daemonPid"].clone();
    f.rpc(json!({"op":"input","id":id,"data":"export TRUFFLE_RESTART_PROOF=still-in-memory\n"}));
    std::thread::sleep(Duration::from_millis(150));
    f.restart("service");
    let after = f.preview();
    assert_eq!(after["sessions"][0]["generation"], generation);
    assert_eq!(after["daemonPid"], daemon);
    f.rpc(json!({"op":"input","id":id,"data":"printf 'PROOF:%s\\n' \"$TRUFFLE_RESTART_PROOF\"\n"}));
    wait(|| {
        let p = f.rpc(json!({"op":"poll","id":id,"cursor":0,"generation":""}));
        let bytes = p["bytes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect::<Vec<_>>();
        String::from_utf8_lossy(&bytes).contains("PROOF:still-in-memory")
    });
    f.restart("terminals");
    let resumed = f.preview();
    assert_ne!(resumed["daemonPid"], daemon);
    assert_ne!(resumed["sessions"][0]["generation"], generation);
    assert_eq!(resumed["sessions"][0]["sessionId"], sid);
    assert_eq!(resumed["sessions"][0]["cwd"], json!(cwd));
}
