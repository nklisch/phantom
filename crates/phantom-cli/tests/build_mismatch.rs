use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::process::Command;

use phantom_core::protocol::{BuildIdentity, DaemonInfo, Request, Response, ResponseData};
use tempfile::tempdir;

#[test]
fn live_build_mismatch_warns_and_preserves_sessions() {
    let dir = tempdir().unwrap();
    let socket = dir.path().join("phantom.sock");
    let listener = UnixListener::bind(&socket).unwrap();

    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap() == 0 {
                break;
            }
            let request: Request = serde_json::from_str(&line).unwrap();
            let response = match request {
                Request::GetDaemonInfo => Response::ok_with(ResponseData::Daemon(DaemonInfo {
                    build: BuildIdentity {
                        version: "0.2.0".into(),
                        commit: "older-build".into(),
                    },
                    running_sessions: 1,
                })),
                Request::ListSessions => Response::ok_with(ResponseData::Sessions(Vec::new())),
                other => panic!("unexpected request: {other:?}"),
            };
            serde_json::to_writer(&mut stream, &response).unwrap();
            stream.write_all(b"\n").unwrap();
        }
    });

    let output = Command::new(env!("CARGO_BIN_EXE_phantom"))
        .arg("--socket")
        .arg(&socket)
        .arg("list")
        .output()
        .unwrap();
    server.join().unwrap();

    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("found daemon build 0.2.0 (older-build)"));
    assert!(stderr.contains("preserving 1 running session(s)"));
}
