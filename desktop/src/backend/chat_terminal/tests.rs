use super::*;
use crate::backend::arr;
use crate::backend::sessions;
use std::path::PathBuf;
#[test]
fn shell_identity_and_working_directory_survive_reopen() {
    let root = tempfile::tempdir().unwrap();
    let repo = Repository::open(root.path().into(), None, true).unwrap();
    let chat = repo
        .transaction(|s| Ok(sessions::fixture(s, root.path(), |_| {})))
        .unwrap();
    let first = open(&repo, &json!({"recordId":chat})).unwrap();
    repo.shells.send(text(&first,"id"),"export TRUFFLE_SHELL_TEST=retained\nprintf 'shell-check:%s\\n' \"$TRUFFLE_SHELL_TEST\"\n").unwrap();
    let second = open(&repo, &json!({"recordId":chat})).unwrap();
    assert_eq!(first["id"], second["id"]);
    assert_eq!(
        PathBuf::from(text(&first, "cwd")),
        root.path().canonicalize().unwrap()
    );
    let mut output = vec![];
    let mut cursor = 0;
    let mut generation = String::new();
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(30));
        let v = repo
            .shells
            .poll(text(&first, "id"), cursor, &generation)
            .unwrap();
        generation = text(&v, "generation").into();
        cursor = v["cursor"].as_u64().unwrap();
        output.extend(
            arr(&v["bytes"])
                .iter()
                .map(|n| u8::try_from(n.as_u64().unwrap()).unwrap()),
        );
        if String::from_utf8_lossy(&output).contains("shell-check:retained") {
            break;
        }
    }
    assert!(String::from_utf8_lossy(&output).contains("shell-check:retained"));
    repo.shells.close(text(&first, "id"), &generation).unwrap();
    assert!(open(&repo, &json!({"recordId":uuid()})).is_err());
}
