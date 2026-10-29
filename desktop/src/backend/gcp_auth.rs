//! Fixed, browser-based login jobs. Credentials and command output never enter app state.
use super::{json, now, uuid, Error, Result, Value};
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
            return json!({"status":"failed","message":"Cannot start gcloud. Install Google Cloud CLI and restart the truffle service."})
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
mod tests;
