use super::*;
use std::os::unix::fs::symlink;

#[test]
fn bounded_reader_rejects_symlinks_and_replaced_file_identity() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join("nested/file.txt"), b"first").unwrap();
    let expected = generation(&fs::metadata(root.join("nested/file.txt")).unwrap());
    assert_eq!(
        read(&root, "nested/file.txt", 5, Some(&expected))
            .unwrap()
            .unwrap(),
        b"first"
    );
    assert!(read(&root, "nested/file.txt", 4, None).is_err());
    fs::write(root.join("replacement.txt"), b"other").unwrap();
    fs::rename(root.join("replacement.txt"), root.join("nested/file.txt")).unwrap();
    assert!(read(&root, "nested/file.txt", 5, Some(&expected)).is_err());
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), b"secret").unwrap();
    symlink(outside.path(), root.join("escape")).unwrap();
    symlink(outside.path().join("secret"), root.join("leaf")).unwrap();
    assert!(read(&root, "escape/secret", 100, None).is_err());
    assert!(read(&root, "leaf", 100, None).is_err());
    assert!(read(&root, "nested", 100, None).is_err());
    assert!(read(&root, "missing/file.txt", 100, None)
        .unwrap()
        .is_none());
    assert!(read(&root, "missing.txt", 100, None).unwrap().is_none());
    for key in [
        "",
        "../secret",
        "/secret",
        "nested/../secret",
        "nested/./file.txt",
        "nested//file.txt",
        "nested/",
    ] {
        assert!(read(&root, key, 100, None).is_err(), "accepted {key}");
    }
}

#[test]
fn reader_rejects_fifo_without_waiting_for_a_writer() {
    use std::{ffi::CString, sync::mpsc, time::Duration};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = CString::new(root.join("pipe").as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let (tx, rx) = mpsc::channel();
    let reader =
        std::thread::spawn(move || tx.send(read(&root, "pipe", 100, None).is_err()).unwrap());
    assert!(rx
        .recv_timeout(Duration::from_secs(2))
        .expect("reader blocked on FIFO"));
    reader.join().unwrap();
}
