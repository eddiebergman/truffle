//! Bounded reads below a trusted root without following child symlinks.
use super::{Error, Result};
use std::{
    fs,
    io::Read,
    path::{Component, Path},
};

pub(crate) fn directory(root: &Path, path: &Path, create: bool) -> std::io::Result<fs::File> {
    use std::{
        ffi::CString,
        os::fd::{AsRawFd, FromRawFd},
        os::unix::fs::OpenOptionsExt,
    };
    let mut current = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root)?;
    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Unsafe local file path.",
            ));
        };
        let name = CString::new(name.as_encoded_bytes()).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Invalid local directory name.",
            )
        })?;
        let open = || unsafe {
            libc::openat(
                current.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        let mut fd = open();
        if fd < 0
            && create
            && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound
        {
            let result = unsafe { libc::mkdirat(current.as_raw_fd(), name.as_ptr(), 0o770) };
            if result < 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(std::io::Error::last_os_error());
            }
            fd = open();
        }
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        current = unsafe { fs::File::from_raw_fd(fd) };
    }
    Ok(current)
}
pub(crate) fn generation(metadata: &fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt;
    format!(
        "{}:{}:{}:{}:{}",
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec()
    )
}
pub(crate) fn read(
    root: &Path,
    key: &str,
    limit: u64,
    expected_generation: Option<&str>,
) -> Result<Option<Vec<u8>>> {
    use std::{
        ffi::CString,
        os::fd::{AsRawFd, FromRawFd},
    };
    let path = Path::new(key);
    if key.is_empty()
        || key
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(Error::new("Unsafe local file path."));
    }
    let parent = match directory(root, path.parent().unwrap(), false) {
        Ok(parent) => parent,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let name = CString::new(path.file_name().unwrap().as_encoded_bytes())
        .map_err(|_| Error::new("Invalid local file path."))?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        return Err(error.into());
    }
    let mut file = unsafe { fs::File::from_raw_fd(fd) };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(Error::new(
            "Local file is not a regular file or exceeds the size limit.",
        ));
    }
    let observed = generation(&metadata);
    if expected_generation.is_some_and(|expected| expected != observed) {
        return Err(Error::new(
            "Local file changed during the scan. Reload the file.",
        ));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if generation(&file.metadata()?) != observed {
        return Err(Error::new("Local file changed while reading."));
    }
    if bytes.len() as u64 > limit {
        return Err(Error::new("Local file exceeded the size limit."));
    }
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
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
}
