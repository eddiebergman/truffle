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
