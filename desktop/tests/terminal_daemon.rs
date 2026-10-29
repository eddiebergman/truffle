//! Regression: replace the entire HTTP process, retain the same PTY and shell memory.
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Owned(Child);
impl Drop for Owned {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn rpc(dir: &Path, p: &Value) -> Value {
    let mut s = UnixStream::connect(dir.join("terminal.sock")).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
    let bytes = serde_json::to_vec(&json!({"protocol":1,"request":p})).unwrap();
    s.write_all(&u32::try_from(bytes.len()).unwrap().to_be_bytes())
        .unwrap();
    s.write_all(&bytes).unwrap();
    let mut n = [0; 4];
    s.read_exact(&mut n).unwrap();
    let n = u32::from_be_bytes(n) as usize;
    assert!(n <= 512 * 1024);
    let mut out = vec![0; n];
    s.read_exact(&mut out).unwrap();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    v["value"].clone()
}
struct FixtureDaemon(libc::pid_t);
impl Drop for FixtureDaemon {
    fn drop(&mut self) {
        unsafe {
            libc::kill(self.0, libc::SIGTERM);
        }
    }
}
fn terminal_socket(data: &Path) -> PathBuf {
    let hash = context_kit::sha256(data.to_string_lossy().as_bytes());
    PathBuf::from(format!(
        "/tmp/truffle-pty-{}-{}",
        unsafe { libc::geteuid() },
        &hash[..16]
    ))
}
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
fn demo_daemon(root: &Path, data: &Path) -> Owned {
    Owned(
        Command::new(env!("CARGO_BIN_EXE_truffle-desktop"))
            .args(["--terminal-daemon", "--allow-demo-terminals", "--home"])
            .arg(root)
            .arg("--data")
            .arg(data)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    )
}
fn wait_for(mut check: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(12);
    while Instant::now() < until {
        if check() {
            return;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    panic!("Test process did not become ready");
}
fn backend(root: &Path, data: &Path, port: u16) -> Owned {
    Owned(
        Command::new(env!("CARGO_BIN_EXE_truffle-desktop"))
            .args(["--serve", "--home"])
            .arg(root)
            .arg("--data")
            .arg(data)
            .arg("--port")
            .arg(port.to_string())
            .env("SENTRY_ORG", "")
            .env("SENTRY_AUTH_TOKEN", "")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    )
}
fn client(timeout: Duration) -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .unwrap()
}
fn get(client: &reqwest::blocking::Client, base: &str, path: &str) -> Value {
    client
        .get(format!("{base}{path}"))
        .send()
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .unwrap()
}
#[test]
fn terminal_and_buffer_survive_backend_replacement() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("state.json");
    std::fs::write(&data,serde_json::to_vec(&json!({"version":1,"sources":{"github":{"disabled":true},"linear":{"disabled":true},"sentry":{"disabled":true}}})).unwrap()).unwrap();
    let dir = terminal_socket(&data.canonicalize().unwrap());
    let mut daemon = demo_daemon(root.path(), &data);
    wait_for(|| dir.join("terminal.sock").exists());
    let id = uuid::Uuid::new_v4().to_string();
    rpc(
        &dir,
        &json!({"op":"demo","record":{"id":id,"title":"Restart regression","cwd":"/tmp","agent":"test"}}),
    );
    rpc(
        &dir,
        &json!({"op":"input","id":id,"data":"export TRUFFLE_RESTART_PROOF=kept-in-the-same-shell\n"}),
    );
    let initial = rpc(
        &dir,
        &json!({"op":"poll","id":id,"cursor":0,"generation":""}),
    );
    let generation = initial["generation"].as_str().unwrap().to_owned();
    let cursor = initial["cursor"].as_u64().unwrap();
    let port = free_port();
    let base = format!("http://127.0.0.1:{port}");
    let client = client(Duration::from_secs(3));
    let mut first = backend(root.path(), &data, port);
    wait_for(|| client.get(format!("{base}/api/health")).send().is_ok());
    assert_eq!(
        get(&client, &base, "/api/agents")["candidates"][0]["generation"],
        generation
    );
    first.0.kill().unwrap();
    first.0.wait().unwrap();
    assert!(daemon.0.try_wait().unwrap().is_none());
    let replacement = backend(root.path(), &data, port);
    wait_for(|| client.get(format!("{base}/api/health")).send().is_ok());
    let agents = get(&client, &base, "/api/agents");
    assert_eq!(agents["candidates"][0]["generation"], generation);
    assert_eq!(agents["candidates"][0]["live"], true);
    let state = get(&client, &base, "/api/state");
    let csrf = state["csrf"].as_str().unwrap();
    let api = |path: &str, p: Value| -> Value {
        client
            .post(format!("{base}{path}"))
            .header("Origin", &base)
            .header("X-CSRF-Token", csrf)
            .json(&p)
            .send()
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .unwrap()
    };
    api(
        "/api/terminal/input",
        json!({"id":id,"data":"printf 'PROOF:%s\\n' \"$TRUFFLE_RESTART_PROOF\"\n"}),
    );
    let mut output = Vec::new();
    let mut next = cursor;
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        let p = api(
            "/api/terminal/poll",
            json!({"id":id,"cursor":next,"generation":generation}),
        );
        assert_eq!(p["generation"], generation);
        next = p["cursor"].as_u64().unwrap();
        output.extend(
            p["bytes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| u8::try_from(v.as_u64().unwrap()).unwrap()),
        );
        if String::from_utf8_lossy(&output).contains("PROOF:kept-in-the-same-shell") {
            break;
        }
    }
    assert!(String::from_utf8_lossy(&output).contains("PROOF:kept-in-the-same-shell"));
    let replay = api(
        "/api/terminal/poll",
        json!({"id":id,"cursor":0,"generation":""}),
    );
    assert_eq!(replay["generation"], generation);
    assert_ne!(
        replay["bytes"].as_array().unwrap().as_slice(),
        [] as [Value; 0]
    );
    api(
        "/api/terminal/close",
        json!({"id":id,"generation":generation}),
    );
    assert_eq!(
        get(&client, &base, "/api/agents")["candidates"][0]["live"],
        false
    );
    // Drop only these owned fixtures, never a production service or terminal daemon.
    drop(replacement);
    drop(daemon);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn normal_launch_lazily_starts_detached_daemon_and_reuses_it() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let data = root.join("state.json");
    let id = uuid::Uuid::new_v4().to_string();
    let sid = uuid::Uuid::new_v4().to_string();
    std::fs::write(&data,serde_json::to_vec(&json!({"version":1,"sessions":[{"id":id,"sessionId":sid,"agent":"claude","title":"Fake agent fixture","cwd":root,"destination":"embedded","launchNew":true,"items":[]}],"sources":{"github":{"disabled":true},"linear":{"disabled":true},"sentry":{"disabled":true}}})).unwrap()).unwrap();
    let bin = root.join("bin");
    std::fs::create_dir(&bin).unwrap();
    std::fs::create_dir(root.join(".claude")).unwrap();
    let fake = bin.join("claude");
    std::fs::write(&fake, "#!/bin/sh\nexec /bin/sh -i\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
    let port = free_port();
    let base = format!("http://127.0.0.1:{port}");
    let mut service = Owned(
        Command::new(env!("CARGO_BIN_EXE_truffle-desktop"))
            .arg("--serve")
            .arg("--home")
            .arg(&root)
            .arg("--data")
            .arg(&data)
            .arg("--port")
            .arg(port.to_string())
            .env("HOME", &root)
            .env("PATH", &bin)
            .env("SENTRY_ORG", "")
            .env("SENTRY_AUTH_TOKEN", "")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let client = client(Duration::from_secs(10));
    wait_for(|| client.get(format!("{base}/api/health")).send().is_ok());
    assert_eq!(
        get(&client, &base, "/api/agents")["candidates"]
            .as_array()
            .unwrap()
            .as_slice(),
        [] as [Value; 0]
    );
    let state = get(&client, &base, "/api/state");
    let response: Value = client
        .post(format!("{base}/api/session/open"))
        .header("Origin", &base)
        .header("X-CSRF-Token", state["csrf"].as_str().unwrap())
        .json(&json!({"id":id}))
        .send()
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(response["openReceipt"]["status"], "accepted", "{response}");
    assert_eq!(response["openReceipt"]["target"], "embedded");
    assert_eq!(response["openReceipt"]["terminal"]["state"], "acknowledged");
    let dir = terminal_socket(&data);
    let health = rpc(&dir, &json!({"op":"health"}));
    let pid = libc::pid_t::try_from(health["pid"].as_u64().unwrap()).unwrap();
    let daemon = FixtureDaemon(pid);
    let agents = get(&client, &base, "/api/agents");
    let generation = agents["candidates"][0]["generation"].clone();
    assert_eq!(agents["candidates"][0]["live"], true);
    service.0.kill().unwrap();
    service.0.wait().unwrap();
    assert_eq!(rpc(&dir, &json!({"op":"health"}))["pid"], pid);
    assert_eq!(
        rpc(&dir, &json!({"op":"inventory"}))[0]["generation"],
        generation
    );
    rpc(&dir, &json!({"op":"close","id":id,"generation":generation}));
    drop(daemon);
    let _ = std::fs::remove_dir_all(&dir);
}
