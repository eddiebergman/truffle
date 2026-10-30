use super::*;
use crate::backend::{arr, Repository};
#[test]
fn close_stops_only_the_matching_terminal_generation_and_allows_resume() {
    let host = EmbeddedHost::default();
    let id = uuid();
    let record = json!({"id":id});
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
    })
    .unwrap();
    // No interactive shell, login scripts, model, or persistent daemon.
    let mut command = CommandBuilder::new("/bin/cat");
    command.cwd(root.path());
    let super::super::terminal_daemon::Host::Local(host) = repo.shells.host() else {
        panic!("Demo repository must use an isolated in-memory terminal host");
    };
    host.spawn(&json!({"id":id}), command).unwrap();
    let generation = text(&repo.shells.inventory().unwrap().rows()[0], "generation").to_owned();
    let outcome = (|| -> Result<()> {
        let input = |data: Value| {
            super::super::chat_terminal::action(
                &repo,
                "/api/shell-terminal/input",
                &json!({"id":id,"data":data}),
            )
        };
        if super::super::chat_terminal::action(
            &repo,
            "/api/shell-terminal/input",
            &json!({"id":id}),
        )
        .is_ok()
            || input(json!(42)).is_ok()
            || input(json!("é".repeat(32769))).is_ok()
        {
            return Err(Error::new(
                "Missing, numeric, or oversized terminal input was accepted.",
            ));
        }
        for packet in [
            "BEGIN",
            " ",
            "middle",
            "\t",
            "tail",
            "\r",
            "  pasted text  ",
            "\r",
        ] {
            input(json!(packet))?;
        }
        let mut bytes = vec![];
        let mut cursor = 0;
        let expected = [
            b"BEGIN middle\ttail\r\n".as_slice(),
            b"  pasted text  \r\n".as_slice(),
        ];
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            let packet = repo.shells.poll(&id, cursor, &generation)?;
            cursor = packet["cursor"].as_u64().unwrap();
            bytes.extend(
                arr(&packet["bytes"])
                    .iter()
                    .map(|n| u8::try_from(n.as_u64().unwrap()).unwrap()),
            );
            // The PTY echoes input too (expanding tabs); cat's returned line
            // retains the actual tab, distinguishing it from that echo.
            if expected
                .iter()
                .all(|line| bytes.windows(line.len()).any(|window| window == *line))
            {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        Err(Error::new(format!(
            "Companion input lost bytes: {:?}",
            String::from_utf8_lossy(&bytes)
        )))
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
        bytes.extend(
            arr(&p["bytes"])
                .iter()
                .map(|x| u8::try_from(x.as_u64().unwrap()).unwrap()),
        );
        if p["alive"] == false && p["hasMore"] == false {
            break;
        }
    }
    assert_eq!(bytes.len(), 100_003);
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
                .map(|x| u8::try_from(x.as_u64().unwrap()).unwrap()),
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
