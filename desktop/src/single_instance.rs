//! One app process per user. A second launch asks the first to show its window and exits.
use std::{
    io::ErrorKind,
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    time::Duration,
};

/// The socket path of tauri-plugin-single-instance, so older and newer builds find each other.
pub fn socket(identifier: &str) -> PathBuf {
    PathBuf::from(format!(
        "/tmp/{}_si.sock",
        identifier.replace(['.', '-'], "_")
    ))
}
pub enum Instance {
    /// Another process accepted the request to show its window.
    Secondary,
    Primary(UnixListener),
    /// The socket is unusable, so this process runs without coordination.
    Uncoordinated,
}
pub fn claim(socket: &Path) -> Instance {
    match UnixStream::connect(socket) {
        Ok(_) => Instance::Secondary,
        Err(error) => match error.kind() {
            ErrorKind::NotFound | ErrorKind::ConnectionRefused => {
                let _ = std::fs::remove_file(socket);
                UnixListener::bind(socket).map_or(Instance::Uncoordinated, Instance::Primary)
            }
            _ => Instance::Uncoordinated,
        },
    }
}
/// Calls `on_request` once for each later launch.
pub fn serve(listener: UnixListener, on_request: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            // Older builds send their working folder and arguments. Read them to the end so
            // their write does not fail and make them start a second window.
            let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
            let _ = std::io::copy(&mut stream, &mut std::io::sink());
            on_request();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    #[test]
    fn a_second_claim_notifies_the_primary_and_a_stale_socket_is_replaced() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("app.sock");
        drop(UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        let Instance::Primary(listener) = claim(&path) else {
            panic!("a stale socket must not block the first instance");
        };
        let (sender, receiver) = mpsc::channel();
        serve(listener, move || sender.send(()).unwrap());
        assert!(matches!(claim(&path), Instance::Secondary));
        receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(claim(&path), Instance::Secondary));
        receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    #[test]
    fn the_socket_path_matches_the_plugin_path() {
        assert_eq!(
            socket("local.truffle-desktop"),
            PathBuf::from("/tmp/local_truffle_desktop_si.sock")
        );
    }
}
