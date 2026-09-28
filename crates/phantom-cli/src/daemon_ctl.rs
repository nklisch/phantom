use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use phantom_core::protocol::{BuildIdentity, Request, Response, ResponseData};
use phantom_core::types::SessionStatus;

use crate::connection::Connection;

static CUSTOM_SOCKET_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn set_socket_path(path: &str) {
    let _ = CUSTOM_SOCKET_PATH.set(PathBuf::from(path));
}

pub fn socket_path() -> PathBuf {
    if let Some(p) = CUSTOM_SOCKET_PATH.get() {
        return p.clone();
    }
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let mut p = dirs_home().unwrap_or_else(|| PathBuf::from("/tmp"));
            p.push(".phantom");
            p
        });
    runtime_dir.join("phantom.sock")
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

/// Remove a stale socket file left behind by a crashed daemon.
/// Returns true if a stale socket was cleaned up.
fn cleanup_stale_socket(path: &PathBuf) -> bool {
    if !path.exists() {
        return false;
    }
    // Try a quick connect — if it fails, the socket is stale
    match std::os::unix::net::UnixStream::connect(path) {
        Ok(_) => false, // daemon is actually running
        Err(_) => {
            let _ = std::fs::remove_file(path);
            true
        }
    }
}

pub async fn ensure_daemon() -> Result<Connection> {
    let path = socket_path();

    if let Ok(mut conn) = Connection::connect(&path).await {
        let expected = build_identity();
        let (actual, running_sessions) = daemon_info(&mut conn).await?;
        match compatibility(&expected, actual.as_ref(), running_sessions) {
            Compatibility::Match => return Ok(conn),
            Compatibility::Warn(message) => {
                eprintln!("warning: {message}");
                return Ok(conn);
            }
            Compatibility::Replace(message) => {
                eprintln!("warning: {message}; restarting idle daemon");
                stop_daemon(&path, &mut conn).await?;
            }
        }
    }

    cleanup_stale_socket(&path);
    start_daemon(&path).await
}

fn build_identity() -> BuildIdentity {
    BuildIdentity {
        version: env!("CARGO_PKG_VERSION").to_string(),
        commit: env!("PHANTOM_BUILD_COMMIT").to_string(),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Compatibility {
    Match,
    Replace(String),
    Warn(String),
}

fn compatibility(
    expected: &BuildIdentity,
    actual: Option<&BuildIdentity>,
    running_sessions: usize,
) -> Compatibility {
    if actual == Some(expected) {
        return Compatibility::Match;
    }

    let mismatch = match actual {
        Some(actual) => format!("phantom client build {expected} found daemon build {actual}"),
        None => format!("phantom client build {expected} found a daemon with no build identity"),
    };
    if running_sessions == 0 {
        Compatibility::Replace(mismatch)
    } else {
        Compatibility::Warn(format!(
            "{mismatch}; preserving {running_sessions} running session(s)"
        ))
    }
}

async fn daemon_info(conn: &mut Connection) -> Result<(Option<BuildIdentity>, usize)> {
    if let Response::Ok {
        data: Some(ResponseData::Daemon(info)),
    } = conn.send(&Request::GetDaemonInfo).await?
    {
        return Ok((Some(info.build), info.running_sessions));
    }

    let running_sessions = match conn.send(&Request::ListSessions).await? {
        Response::Ok {
            data: Some(ResponseData::Sessions(sessions)),
        } => sessions
            .iter()
            .filter(|session| matches!(session.status, SessionStatus::Running))
            .count(),
        response => bail!("daemon did not report build or sessions: {response:?}"),
    };
    Ok((None, running_sessions))
}

async fn stop_daemon(path: &PathBuf, conn: &mut Connection) -> Result<()> {
    let peer_pid = conn.peer_pid();
    let _ = conn.send(&Request::Shutdown).await;

    for attempt in 0..40 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if Connection::connect(path).await.is_err() {
            cleanup_stale_socket(path);
            return Ok(());
        }
        if attempt == 9
            && let Some(pid) = peer_pid
        {
            let _ = kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
        }
    }
    bail!("daemon at {} did not stop", path.display())
}

async fn start_daemon(path: &PathBuf) -> Result<Connection> {
    let daemon_bin = std::env::current_exe()?
        .parent()
        .context("cannot determine executable directory")?
        .join("phantom-daemon");

    if !daemon_bin.exists() {
        bail!(
            "Daemon binary not found at {path}\n\
             \n\
             Make sure phantom-daemon is built and in the same directory as phantom:\n\
             \n\
                 cargo build --workspace",
            path = daemon_bin.display()
        );
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create socket directory: {}", parent.display()))?;
    }

    Command::new(&daemon_bin)
        .arg("--socket")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Failed to start daemon: {}", daemon_bin.display()))?;

    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if let Ok(mut conn) = Connection::connect(path).await
            && let Ok((Some(actual), _)) = daemon_info(&mut conn).await
            && actual == build_identity()
        {
            return Ok(conn);
        }
    }

    bail!(
        "Daemon failed to start within 5 seconds\n\
         \n\
         Try running it manually to see errors:\n\
         \n\
             phantom daemon start --foreground"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(version: &str, commit: &str) -> BuildIdentity {
        BuildIdentity {
            version: version.to_string(),
            commit: commit.to_string(),
        }
    }

    #[test]
    fn matching_build_is_used() {
        let expected = build("0.3.0", "abc");
        assert_eq!(
            compatibility(&expected, Some(&expected), 0),
            Compatibility::Match
        );
    }

    #[test]
    fn idle_mismatched_build_is_replaced() {
        let expected = build("0.3.0", "new");
        let actual = build("0.3.0", "old");
        assert!(matches!(
            compatibility(&expected, Some(&actual), 0),
            Compatibility::Replace(message) if message.contains("old")
        ));
    }

    #[test]
    fn live_mismatched_build_is_preserved_with_warning() {
        let expected = build("0.3.0", "new");
        let actual = build("0.3.0", "old");
        assert!(matches!(
            compatibility(&expected, Some(&actual), 2),
            Compatibility::Warn(message)
                if message.contains("preserving 2 running session(s)")
        ));
    }
}
