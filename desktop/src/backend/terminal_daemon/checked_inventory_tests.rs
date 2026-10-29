use super::*;
use crate::backend::uuid;
#[test]
fn runtime_inventory_failure_is_per_call_even_when_shared_warning_is_clear() {
    let temp = tempfile::tempdir().unwrap();
    let remote = Remote {
        root: temp.path().into(),
        data: temp.path().join("state.json"),
        dir: temp.path().into(),
        opening: Mutex::new(()),
        warning: Mutex::new(None),
    };
    let host = Host::Remote(remote);
    let failure = host.inventory_checked();
    assert!(failure.is_err());
    assert!(host.warning().is_none());
    assert_eq!(host.inventory(), [] as [serde_json::Value; 0]);
    assert!(failure.is_err());
    assert!(!temp.path().join("terminal.sock").exists());
}
#[test]
fn runtime_inventory_checked_rejects_invalid_reply_and_preserves_exact_generation() {
    for value in [
        json!({"bad":"shape"}),
        json!([{"host":"embedded","recordId":uuid(),"agent":"codex","sessionId":null,"generation":uuid(),"live":true}]),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(temp.path().join("terminal.sock")).unwrap();
        let expected = value.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_frame(&mut stream).unwrap();
            assert_eq!(request["request"]["op"], "inventory");
            write_frame(
                &mut stream,
                &json!({"protocol":PROTOCOL,"ok":true,"value":value}),
            )
            .unwrap();
        });
        let host = Host::Remote(Remote {
            root: temp.path().into(),
            data: temp.path().join("state.json"),
            dir: temp.path().into(),
            opening: Mutex::new(()),
            warning: Mutex::new(None),
        });
        let result = host.inventory_checked();
        server.join().unwrap();
        if expected.is_array() {
            assert_eq!(json!(result.unwrap()), expected);
        } else {
            assert!(result.is_err());
        }
    }
}
