//! Fixed, browser-based login jobs. Credentials and command output never enter app state.
use super::*;
use std::{
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use wait_timeout::ChildExt;

#[derive(Default)]
pub struct LoginHost {
    state: Mutex<LoginState>,
}
#[derive(Default)]
struct LoginState {
    id: String,
    started: f64,
    cli: Option<Value>,
    adc: Option<Value>,
    cancel: Arc<AtomicBool>,
}
impl LoginState {
    fn running(&self) -> bool {
        [&self.cli, &self.adc]
            .iter()
            .any(|task| task.as_ref().is_some_and(|v| v["status"] == "running"))
    }
    fn view(&self) -> Value {
        json!({"runId":self.id,"running":self.running(),"cancelling":self.cancel.load(Ordering::Relaxed),"startedAt":self.started,"cooldownUntil":if self.started>0.0{self.started+30.0}else{0.0},"cli":self.cli,"adc":self.adc})
    }
}
impl LoginHost {
    pub fn view(&self) -> Value {
        self.state.lock().unwrap().view()
    }
    pub fn start(self: &Arc<Self>, demo: bool) -> Result<Value> {
        self.launch(
            super::terminal::executable("gcloud"),
            demo,
            Duration::from_secs(600),
        )
    }
    pub fn cancel(&self) -> Value {
        let state = self.state.lock().unwrap();
        if state.running() {
            state.cancel.store(true, Ordering::Relaxed);
        }
        state.view()
    }
    fn launch(
        self: &Arc<Self>,
        executable: String,
        demo: bool,
        timeout: Duration,
    ) -> Result<Value> {
        let mut state = self.state.lock().unwrap();
        if state.running() {
            return Ok(state.view());
        }
        if now() < state.started + 30.0 {
            return Err(Error::new(
                "GCP login is cooling down. Try again in a few seconds.",
            ));
        }
        *state = LoginState {
            id: uuid(),
            started: now(),
            cli: Some(json!({"status":"running","message":"Complete CLI login in your browser."})),
            adc: Some(json!({"status":"running","message":"Complete ADC login in your browser."})),
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let id = state.id.clone();
        let cancel = state.cancel.clone();
        let initial = state.view();
        drop(state);
        for adc in [false, true] {
            let host = self.clone();
            let id = id.clone();
            let cancel = cancel.clone();
            let executable = executable.clone();
            std::thread::spawn(move || {
                let task = if demo {
                    json!({"status":"succeeded","message":"Demo completed; no credentials changed."})
                } else {
                    login(&executable, adc, timeout, &cancel)
                };
                let mut state = host.state.lock().unwrap();
                if state.id == id {
                    if adc {
                        state.adc = Some(task);
                    } else {
                        state.cli = Some(task);
                    }
                }
            });
        }
        Ok(initial)
    }
}
fn login(executable: &str, adc: bool, timeout: Duration, cancel: &AtomicBool) -> Value {
    let mut command = Command::new(executable);
    command.arg("auth");
    if adc {
        command.arg("application-default");
    }
    command.arg("login").args(["--quiet", "--launch-browser"]);
    // A refresh button should open the CLI browser flow even when cached credentials remain valid.
    if !adc {
        command.arg("--force");
    }
    command.env_remove("CLOUDSDK_AUTH_ACCESS_TOKEN");
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if cancel.load(Ordering::Relaxed) {
        return json!({"status":"cancelled","message":"Login cancelled."});
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            return json!({"status":"failed","message":"Cannot start gcloud. Install Google Cloud CLI and restart the mydash service."})
        }
    };
    let started = std::time::Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) || started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return if cancel.load(Ordering::Relaxed) {
                json!({"status":"cancelled","message":"Login cancelled."})
            } else {
                json!({"status":"timed_out","message":"Login timed out after 10 minutes. Retry when ready to finish in your browser."})
            };
        }
        match child.wait_timeout(Duration::from_millis(200)) {
            Ok(Some(status)) => {
                return if status.success() {
                    json!({"status":"succeeded","message":"Signed in."})
                } else {
                    json!({"status":"failed","exitCode":status.code(),"message":if adc{"ADC login failed. Run gcloud auth application-default login in a terminal for details."}else{"CLI login failed. Run gcloud auth login in a terminal for details."}})
                }
            }
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return json!({"status":"failed","message":"Could not monitor login. Retry from a terminal."});
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn script(root: &std::path::Path, body: &str) -> String {
        let p = root.join("gcloud");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
        p.to_string_lossy().into()
    }
    fn finished(host: &LoginHost) -> Value {
        for _ in 0..200 {
            let v = host.view();
            if v["running"] != true {
                return v;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("Login did not finish")
    }
    #[test]
    fn concurrent_logins_use_fixed_arguments_and_do_not_expose_output() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().to_string_lossy();
        let program=script(t.path(),&format!("case \"$2\" in application-default) name=adc;; *) name=cli;; esac\nprintf '%s\\n' \"$@\" > '{root}/'$name.args\ntouch '{root}/'$name.started\nwhile [ ! -f '{root}/release' ]; do sleep 0.01; done\nprintf 'SECRET_TOKEN_NEVER_LOGGED'\nexit 0"));
        let host = Arc::new(LoginHost::default());
        let first = host
            .launch(program.clone(), false, Duration::from_secs(2))
            .unwrap();
        let again = host.launch(program, false, Duration::from_secs(2)).unwrap();
        assert_eq!(first["runId"], again["runId"]);
        for _ in 0..100 {
            if t.path().join("cli.started").exists() && t.path().join("adc.started").exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(t.path().join("cli.started").exists() && t.path().join("adc.started").exists());
        std::fs::write(t.path().join("release"), b"").unwrap();
        let v = finished(&host);
        assert_eq!(v["cli"]["status"], "succeeded");
        assert_eq!(v["adc"]["status"], "succeeded");
        assert!(!v.to_string().contains("SECRET_TOKEN"));
        assert_eq!(
            std::fs::read_to_string(t.path().join("cli.args")).unwrap(),
            "auth\nlogin\n--quiet\n--launch-browser\n--force\n"
        );
        assert_eq!(
            std::fs::read_to_string(t.path().join("adc.args")).unwrap(),
            "auth\napplication-default\nlogin\n--quiet\n--launch-browser\n"
        );
        assert!(host.start(true).is_err());
    }
    #[test]
    fn login_failures_are_independent() {
        let t = tempfile::tempdir().unwrap();
        let p = script(
            t.path(),
            "case \"$2\" in application-default) exit 7;; *) exit 0;; esac",
        );
        let host = Arc::new(LoginHost::default());
        host.launch(p, false, Duration::from_secs(2)).unwrap();
        let v = finished(&host);
        assert_eq!(v["cli"]["status"], "succeeded");
        assert_eq!(v["adc"]["exitCode"], 7);
    }
    #[test]
    fn timeout_and_cancel_reap_background_commands() {
        let t = tempfile::tempdir().unwrap();
        let p = script(t.path(), "while :; do sleep 0.01; done");
        let host = Arc::new(LoginHost::default());
        host.launch(p.clone(), false, Duration::from_millis(50))
            .unwrap();
        let v = finished(&host);
        assert_eq!(v["cli"]["status"], "timed_out");
        assert_eq!(v["adc"]["status"], "timed_out");
        let host = Arc::new(LoginHost::default());
        host.launch(p, false, Duration::from_secs(2)).unwrap();
        host.cancel();
        let v = finished(&host);
        assert_eq!(v["cli"]["status"], "cancelled");
        assert_eq!(v["adc"]["status"], "cancelled");
    }
}
