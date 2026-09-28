use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use phantom_core::protocol::{Request, Response, ResponseData};
use tempfile::tempdir;

fn spawn_daemon(socket: &Path, extra_args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_phantom-daemon"))
        .arg("--foreground")
        .arg("--socket")
        .arg(socket)
        .args(extra_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn wait_for_listener(socket: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if UnixStream::connect(socket).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("daemon did not listen at {}", socket.display());
}

fn wait_for_exit(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let _ = child.kill();
    panic!("daemon did not exit");
}

fn request(socket: &Path, request: &Request) -> Response {
    let mut stream = UnixStream::connect(socket).unwrap();
    serde_json::to_writer(&mut stream, request).unwrap();
    stream.write_all(b"\n").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

#[test]
fn daemon_reports_its_build_identity() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("phantom.sock");
    let mut daemon = spawn_daemon(&socket, &["--idle-timeout-secs", "0"]);
    wait_for_listener(&socket);

    let response = request(&socket, &Request::GetDaemonInfo);
    match response {
        Response::Ok {
            data: Some(ResponseData::Daemon(info)),
        } => {
            assert_eq!(info.build.version, env!("CARGO_PKG_VERSION"));
            assert_eq!(info.build.commit, env!("PHANTOM_BUILD_COMMIT"));
            assert_eq!(info.running_sessions, 0);
        }
        other => panic!("unexpected daemon info response: {other:?}"),
    }

    assert!(matches!(
        request(&socket, &Request::Shutdown),
        Response::Ok { .. }
    ));
    wait_for_exit(&mut daemon);
    assert!(!socket.exists(), "daemon should remove its socket on exit");
}

#[test]
fn daemon_replaces_a_stale_socket_file() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("phantom.sock");
    std::fs::write(&socket, b"stale").unwrap();

    let mut daemon = spawn_daemon(&socket, &["--idle-timeout-secs", "0"]);
    wait_for_listener(&socket);
    assert!(matches!(
        request(&socket, &Request::GetDaemonInfo),
        Response::Ok {
            data: Some(ResponseData::Daemon(_))
        }
    ));

    let _ = request(&socket, &Request::Shutdown);
    wait_for_exit(&mut daemon);
}

#[test]
fn daemon_exits_when_its_owner_exits() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("phantom.sock");
    let mut owner = Command::new("sleep").arg("0.5").spawn().unwrap();
    let owner_arg = owner.id().to_string();
    let mut daemon = spawn_daemon(
        &socket,
        &["--owner-pid", &owner_arg, "--idle-timeout-secs", "0"],
    );
    wait_for_listener(&socket);

    assert!(owner.wait().unwrap().success());
    wait_for_exit(&mut daemon);
    assert!(!socket.exists(), "owner exit should clean up the socket");
}

#[test]
fn daemon_exits_after_the_configured_idle_period() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("phantom.sock");
    let mut daemon = spawn_daemon(&socket, &["--idle-timeout-secs", "1"]);
    wait_for_listener(&socket);

    wait_for_exit(&mut daemon);
    assert!(!socket.exists(), "idle exit should clean up the socket");
}
