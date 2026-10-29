use super::*;
#[test]
fn socket_directory_identity_uses_canonical_parent_and_rejects_public_or_symlink_directories() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().canonicalize().unwrap();
    assert_eq!(
        socket_directory(&root.path().join("state.json")).unwrap(),
        socket_directory(&parent.join("state.json")).unwrap()
    );
    let private = root.path().join("private");
    private_directory(&private).unwrap();
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(private_directory(&private).is_err());
    let link = root.path().join("link");
    std::os::unix::fs::symlink(&private, &link).unwrap();
    assert!(private_directory(&link).is_err());
}
#[test]
fn socket_directory_keeps_a_live_daemon_from_before_the_rename() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("state.json");
    let current = socket_directory(&data).unwrap();
    let legacy = PathBuf::from(
        current
            .to_string_lossy()
            .replace("/tmp/truffle-pty-", "/tmp/mydash-pty-"),
    );
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("terminal.sock"), "").unwrap();
    assert_eq!(socket_directory(&data).unwrap(), legacy);
    std::fs::create_dir_all(&current).unwrap();
    std::fs::write(current.join("terminal.sock"), "").unwrap();
    assert_eq!(socket_directory(&data).unwrap(), current);
    std::fs::remove_dir_all(&legacy).unwrap();
    std::fs::remove_dir_all(&current).unwrap();
}
#[test]
fn framing_and_dispatch_bound_untrusted_messages() {
    let (mut writer, mut reader) = UnixStream::pair().unwrap();
    writer
        .write_all(&((FRAME_LIMIT + 1) as u32).to_be_bytes())
        .unwrap();
    assert!(read_frame(&mut reader).is_err());
    let root = tempfile::tempdir().unwrap();
    let host = super::super::embedded::EmbeddedHost::default();
    assert!(dispatch(
        &host,
        root.path(),
        &root.path().join("state.json"),
        false,
        &json!({"op":"demo","record":{"id":uuid()}})
    )
    .is_err());
    assert!(dispatch(
        &host,
        root.path(),
        &root.path().join("state.json"),
        false,
        &json!({"op":"resize","id":uuid(),"cols":65537,"rows":30})
    )
    .is_err());
    assert!(dispatch(
        &host,
        root.path(),
        &root.path().join("state.json"),
        false,
        &json!({"op":"run","command":"anything"})
    )
    .is_err());
}
