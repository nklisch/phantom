use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use crossbeam_channel::Sender;
use mio::Waker;
use nix::sys::signal::kill;
use nix::unistd::Pid;
use phantom_core::protocol::BuildIdentity;
use tokio::net::UnixListener;

use crate::engine::EngineCommand;
use crate::handler;

pub struct Lifecycle {
    pub owner_pid: Option<u32>,
    pub idle_timeout: Option<Duration>,
}

pub async fn listen(
    socket_path: &Path,
    cmd_tx: Sender<EngineCommand>,
    waker: Arc<Waker>,
    build: BuildIdentity,
    lifecycle: Lifecycle,
) -> Result<()> {
    let listener = bind(socket_path)?;
    tracing::info!("Listening on {}", socket_path.display());

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel(1);
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    let mut idle_since = Instant::now();

    loop {
        tokio::select! {
            result = listener.accept() => match result {
                Ok((stream, _addr)) => {
                    let cmd_tx = cmd_tx.clone();
                    let waker = Arc::clone(&waker);
                    let build = build.clone();
                    let shutdown_tx = shutdown_tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handler::handle_connection(
                            stream,
                            cmd_tx,
                            waker,
                            build,
                            Some(shutdown_tx),
                        ).await {
                            tracing::warn!("Connection error: {e}");
                        }
                    });
                }
                Err(e) => tracing::error!("Accept error: {e}"),
            },
            _ = shutdown_rx.recv() => {
                tracing::info!("Shutdown requested by client");
                return Ok(());
            }
            _ = interval.tick() => {
                if lifecycle.owner_pid.is_some_and(|pid| !process_exists(pid)) {
                    tracing::info!("Owner process exited");
                    return Ok(());
                }

                if let Some(timeout) = lifecycle.idle_timeout {
                    if running_session_count(&cmd_tx, &waker).await? == 0 {
                        if idle_since.elapsed() >= timeout {
                            tracing::info!("Idle timeout reached");
                            return Ok(());
                        }
                    } else {
                        idle_since = Instant::now();
                    }
                }
            }
        }
    }
}

pub fn bind(socket_path: &Path) -> Result<UnixListener> {
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if socket_path.exists() {
        match std::os::unix::net::UnixStream::connect(socket_path) {
            Ok(_) => bail!("socket {} already has a listener", socket_path.display()),
            Err(e) if matches!(e.kind(), ErrorKind::ConnectionRefused | ErrorKind::NotFound) => {
                std::fs::remove_file(socket_path)
                    .with_context(|| format!("removing stale socket {}", socket_path.display()))?;
            }
            Err(e) => {
                return Err(e).with_context(|| {
                    format!("checking existing socket {}", socket_path.display())
                });
            }
        }
    }

    UnixListener::bind(socket_path)
        .with_context(|| format!("binding socket at {}", socket_path.display()))
}

async fn running_session_count(cmd_tx: &Sender<EngineCommand>, waker: &Waker) -> Result<usize> {
    let (reply_tx, reply_rx) = crossbeam_channel::bounded(1);
    cmd_tx.send(EngineCommand::RunningSessionCount { reply: reply_tx })?;
    waker.wake()?;
    let count = tokio::task::spawn_blocking(move || reply_rx.recv_timeout(Duration::from_secs(1)))
        .await??;
    Ok(count)
}

fn process_exists(pid: u32) -> bool {
    match kill(Pid::from_raw(pid as i32), None) {
        Ok(()) | Err(nix::errno::Errno::EPERM) => true,
        Err(nix::errno::Errno::ESRCH) => false,
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn bind_replaces_a_stale_socket() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("phantom.sock");
        std::fs::write(&path, b"stale").unwrap();

        let listener = bind(&path).unwrap();
        let client = tokio::net::UnixStream::connect(&path).await.unwrap();
        let (_server, _) = listener.accept().await.unwrap();
        drop(client);
    }

    #[tokio::test]
    async fn bind_does_not_replace_a_live_socket() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("phantom.sock");
        let _listener = UnixListener::bind(&path).unwrap();

        let error = bind(&path).unwrap_err();
        assert!(error.to_string().contains("already has a listener"));
    }
}
