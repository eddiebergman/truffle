//! Git reads. Each call has a bounded wait, so a stuck helper or lock cannot hold a request.
use super::terminal::{self, Output};
use std::{path::Path, time::Duration};

/// Every call is a local read; none of them reaches the network.
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub(super) enum Error {
    TimedOut,
    Failed(super::Error),
}
impl From<Error> for super::Error {
    fn from(error: Error) -> Self {
        match error {
            Error::TimedOut => terminal::timed_out("git", TIMEOUT),
            Error::Failed(error) => error,
        }
    }
}
/// Runs `git -C root args`. Optional locks stay off, so a read never blocks the user's own Git.
pub(super) fn bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>, Error> {
    let mut command = vec![
        "--no-optional-locks".into(),
        "-C".into(),
        root.to_string_lossy().into_owned(),
    ];
    command.extend(args.iter().map(|arg| (*arg).to_owned()));
    match terminal::bounded("git", &command, None, TIMEOUT).map_err(Error::Failed)? {
        Output::TimedOut => Err(Error::TimedOut),
        Output::Exited(status, stdout, _) if status.success() => Ok(stdout),
        Output::Exited(_, _, stderr) => Err(Error::Failed(terminal::failed(&stderr))),
    }
}
pub(super) fn run(root: &Path, args: &[&str]) -> Result<String, Error> {
    bytes(root, args).map(|bytes| String::from_utf8_lossy(&bytes).into())
}

#[cfg(test)]
mod tests;
