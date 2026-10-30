use super::*;
use std::time::Instant;
#[test]
fn a_stuck_git_command_is_killed_with_a_timeout_error() {
    let temp = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let result = run(temp.path(), &["-c", "alias.stall=!sleep 30", "stall"]);
    assert!(matches!(result, Err(Error::TimedOut)), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(20));
    let error = crate::backend::Error::from(result.unwrap_err());
    assert_eq!(error.data["kind"], "timeout");
}
