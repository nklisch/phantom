use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use tempfile::tempdir;

fn spawn_mcp(socket: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_phantom-mcp"))
        .env("PHANTOM_MCP_SOCKET", socket)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn wait_for_socket(socket: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if socket.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("phantom-mcp did not create {}", socket.display());
}

fn wait_for_exit(child: &mut Child) -> std::process::ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let _ = child.kill();
    panic!("phantom-mcp did not exit");
}

#[test]
fn stdin_eof_stops_engine_and_removes_socket() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("phantom-mcp.sock");
    let mut child = spawn_mcp(&socket);
    wait_for_socket(&socket);

    drop(child.stdin.take());
    let status = wait_for_exit(&mut child);
    assert!(status.success(), "stdin EOF should be a clean exit");
    assert!(
        !socket.exists(),
        "stdin EOF should remove the observer socket"
    );
}

#[test]
fn sigterm_stops_engine_and_removes_socket() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("phantom-mcp.sock");
    let mut child = spawn_mcp(&socket);
    wait_for_socket(&socket);
    std::thread::sleep(Duration::from_millis(100));

    kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM).unwrap();
    let status = wait_for_exit(&mut child);
    assert!(status.success(), "SIGTERM should be a clean exit");
    assert!(
        !socket.exists(),
        "SIGTERM should remove the observer socket"
    );
}
