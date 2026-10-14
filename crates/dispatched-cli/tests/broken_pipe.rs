//! `dispatched … | head` must end quietly when the reader goes away.

#![cfg(unix)]

use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};

#[test]
fn closed_stdout_ends_quietly_instead_of_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_dispatched"))
        .args(["profile", "ls", "--format", "json"])
        .env("DISPATCHED_CONFIG_DIR", dir.path())
        .env_remove("DISPATCHED_CONNECTION_STRING")
        .env_remove("DISPATCHED_NAMESPACE")
        .env_remove("DISPATCHED_PROFILE")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Close the read end before the child writes anything.
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("panicked"), "stderr: {stderr}");
    // Either it finished before noticing, or SIGPIPE ended it.
    assert!(
        out.status.success() || out.status.signal() == Some(libc::SIGPIPE),
        "status: {:?}, stderr: {stderr}",
        out.status
    );
}
