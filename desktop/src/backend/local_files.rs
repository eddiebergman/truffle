//! Bounded reads below a trusted root without following child symlinks.
use super::{Error, Result};
use std::{
    fs,
    io::Read,
    path::{Component, Path},
};

pub fn directory(root: &Path, path: &Path, create: bool) -> std::io::Result<fs::File> {
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
pub fn generation(metadata: &fs::Metadata) -> String {
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
pub fn read(
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
mod tests;
