use super::*;
use crate::backend::uuid;
#[test]
fn inventory_without_a_socket_is_empty_and_a_stale_socket_fails() {
    let temp = tempfile::tempdir().unwrap();
    let host = Host::Remote(Remote {
        root: temp.path().into(),
        data: temp.path().join("state.json"),
        dir: temp.path().into(),
        opening: Mutex::new(()),
    });
    assert_eq!(host.inventory().unwrap(), [] as [serde_json::Value; 0]);
    drop(UnixListener::bind(temp.path().join("terminal.sock")).unwrap());
    assert!(host.inventory().is_err());
}
#[test]
fn runtime_inventory_rejects_invalid_reply_and_preserves_exact_generation() {
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
        });
        let result = host.inventory();
        server.join().unwrap();
        if expected.is_array() {
            assert_eq!(json!(result.unwrap()), expected);
        } else {
            assert!(result.is_err());
        }
    }
}
