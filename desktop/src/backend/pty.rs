//! Pseudo-terminals. Each child leads a new session with the terminal as its controlling terminal.
use std::{
    ffi::CStr,
    fs::File,
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::process::CommandExt,
    },
    process::{Child, Command, Stdio},
};

#[derive(Clone, Copy)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}
impl Size {
    const fn winsize(self) -> libc::winsize {
        libc::winsize {
            ws_row: self.rows,
            ws_col: self.cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        }
    }
}
/// The controlling side of a terminal.
pub struct Terminal {
    master: File,
}
impl Terminal {
    /// Starts `command` on a new terminal. Its standard streams are the terminal.
    pub fn spawn(mut command: Command, size: Size) -> io::Result<(Self, Child)> {
        let (master, slave) = open(size)?;
        command.env("SHELL", shell());
        command
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave));
        // The descriptor scan allocates after fork. The other calls are async-signal-safe.
        unsafe { command.pre_exec(prepare_child) };
        let child = command.spawn()?;
        // The command keeps the parent's copies of the slave until it is dropped.
        drop(command);
        Ok((
            Self {
                master: File::from(master),
            },
            child,
        ))
    }
    pub fn reader(&self) -> io::Result<Reader> {
        Ok(Reader(self.master.try_clone()?))
    }
    pub fn writer(&self) -> io::Result<Writer> {
        Ok(Writer(self.master.try_clone()?))
    }
    pub fn resize(&self, size: Size) -> io::Result<()> {
        let size = size.winsize();
        check(unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &raw const size) })
    }
}
/// Sends SIGHUP, as a terminal does when it closes. The child can save its state and exit.
pub fn hang_up(child: u32) -> io::Result<()> {
    let pid =
        libc::pid_t::try_from(child).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    check(unsafe { libc::kill(pid, libc::SIGHUP) })
}
pub struct Reader(File);
impl Read for Reader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self.0.read(buffer) {
            // macOS reports EIO once every slave descriptor has closed.
            Err(error) if error.raw_os_error() == Some(libc::EIO) => Ok(0),
            result => result,
        }
    }
}
pub struct Writer(File);
impl Write for Writer {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.write(buffer)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}
impl Drop for Writer {
    /// Sends end-of-file to the child. The terminal reads EOF only at the start of a line.
    fn drop(&mut self) {
        let mut attributes = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(self.0.as_raw_fd(), &raw mut attributes) } == 0 {
            let eof = attributes.c_cc[libc::VEOF];
            if eof != 0 {
                let _ = self.0.write_all(&[b'\n', eof]);
            }
        }
    }
}

fn check(result: libc::c_int) -> io::Result<()> {
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
fn open(size: Size) -> io::Result<(OwnedFd, OwnedFd)> {
    let (mut master, mut slave): (RawFd, RawFd) = (-1, -1);
    let mut size = size.winsize();
    check(unsafe {
        libc::openpty(
            &raw mut master,
            &raw mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &raw mut size,
        )
    })?;
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    for fd in [&master, &slave] {
        check(unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) })?;
    }
    Ok((master, slave))
}
/// `$SHELL` when it is executable, then the account's login shell, then /bin/sh.
fn shell() -> String {
    let executable = |path: &str| {
        std::ffi::CString::new(path)
            .is_ok_and(|path| unsafe { libc::access(path.as_ptr(), libc::X_OK) } == 0)
    };
    if let Ok(shell) = std::env::var("SHELL") {
        if executable(&shell) {
            return shell;
        }
    }
    let mut entry = unsafe { std::mem::zeroed::<libc::passwd>() };
    let mut found = std::ptr::null_mut();
    let mut buffer = vec![0; 4096];
    let status = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            &raw mut entry,
            buffer.as_mut_ptr(),
            buffer.len(),
            &raw mut found,
        )
    };
    if status == 0 && !found.is_null() && !entry.pw_shell.is_null() {
        if let Ok(shell) = unsafe { CStr::from_ptr(entry.pw_shell) }.to_str() {
            if executable(shell) {
                return shell.to_owned();
            }
        }
    }
    "/bin/sh".into()
}
fn prepare_child() -> io::Result<()> {
    for signal in [
        libc::SIGCHLD,
        libc::SIGHUP,
        libc::SIGINT,
        libc::SIGQUIT,
        libc::SIGTERM,
        libc::SIGALRM,
    ] {
        unsafe { libc::signal(signal, libc::SIG_DFL) };
    }
    let mut empty = unsafe { std::mem::zeroed::<libc::sigset_t>() };
    unsafe {
        libc::sigemptyset(&raw mut empty);
        libc::sigprocmask(libc::SIG_SETMASK, &raw const empty, std::ptr::null_mut());
    }
    check(unsafe { libc::setsid() })?;
    // Without a controlling terminal the child gets no SIGWINCH on resize.
    check(unsafe { libc::ioctl(0, libc::c_ulong::from(libc::TIOCSCTTY), 0) })?;
    close_inherited_descriptors();
    Ok(())
}
/// Closes every descriptor above stderr. macOS frameworks leak descriptors without CLOEXEC.
fn close_inherited_descriptors() {
    let Ok(entries) = std::fs::read_dir("/dev/fd") else {
        return;
    };
    let descriptors = entries
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<RawFd>().ok())
        .filter(|fd| *fd > 2)
        .collect::<Vec<_>>();
    for fd in descriptors {
        unsafe { libc::close(fd) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    fn read_until(reader: &mut Reader, needle: &str) -> String {
        let mut output = Vec::new();
        let mut buffer = [0; 4096];
        while !String::from_utf8_lossy(&output).contains(needle) {
            let count = reader.read(&mut buffer).unwrap();
            assert_ne!(count, 0, "{}", String::from_utf8_lossy(&output));
            output.extend(&buffer[..count]);
        }
        String::from_utf8_lossy(&output).into_owned()
    }
    #[test]
    fn child_leads_a_session_on_its_controlling_terminal_with_only_standard_descriptors() {
        let root = tempfile::tempdir().unwrap();
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "trap 'echo WINCH:$(stty size)' WINCH; \
                 echo \"ID:$$:$(ps -o pgid= -o tpgid= -o tt= -p $$ | xargs)\"; \
                 echo \"CWD:$(pwd -P) ENV:$TRUFFLE_TEST SHELL:$SHELL\"; \
                 /usr/sbin/lsof -a -p $$ -d 0-99 -F f > fds; echo \"FDS:$(grep ^f fds | xargs)\"; \
                 echo READY; read line; echo \"LINE:$line\"",
            ])
            .current_dir(root.path())
            .env("TRUFFLE_TEST", "value");
        // A descriptor without close-on-exec, like those macOS frameworks leak.
        let leaked = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY) };
        assert!(leaked > 2);
        let spawned = Terminal::spawn(command, Size { cols: 80, rows: 24 });
        unsafe { libc::close(leaked) };
        let (terminal, mut child) = spawned.unwrap();
        let mut reader = terminal.reader().unwrap();
        let output = read_until(&mut reader, "READY");
        let pid = child.id();
        let id_line = output.lines().find(|l| l.starts_with("ID:")).unwrap();
        let fields = id_line
            .trim_start_matches("ID:")
            .split([':', ' '])
            .collect::<Vec<_>>();
        // The child leads its own process group, which is the terminal's foreground group.
        assert_eq!(fields[0], pid.to_string(), "{output}");
        assert_eq!(fields[1], pid.to_string(), "{output}");
        assert_eq!(fields[2], pid.to_string(), "{output}");
        assert!(fields[3].starts_with('s'), "{output}");
        let cwd = root.path().canonicalize().unwrap();
        assert!(
            output.contains(&format!("CWD:{} ENV:value SHELL:/", cwd.display())),
            "{output}"
        );
        assert!(output.contains("FDS:f0 f1 f2\r"), "{output}");
        terminal.resize(Size { cols: 91, rows: 33 }).unwrap();
        read_until(&mut reader, "WINCH:33 91");
        let mut writer = terminal.writer().unwrap();
        writer.write_all(b"typed\n").unwrap();
        read_until(&mut reader, "LINE:typed");
        assert!(child.wait().unwrap().success());
    }
    #[test]
    fn dropping_the_writer_sends_end_of_file_and_hang_up_stops_the_child() {
        let (terminal, mut cat) =
            Terminal::spawn(Command::new("/bin/cat"), Size { cols: 80, rows: 24 }).unwrap();
        drop(terminal.writer().unwrap());
        assert!(cat.wait().unwrap().success());
        let (_terminal, mut sleeper) = Terminal::spawn(
            {
                let mut command = Command::new("/bin/sleep");
                command.arg("30");
                command
            },
            Size { cols: 80, rows: 24 },
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(50));
        hang_up(sleeper.id()).unwrap();
        let status = sleeper.wait().unwrap();
        assert_eq!(
            std::os::unix::process::ExitStatusExt::signal(&status),
            Some(libc::SIGHUP)
        );
    }
}
