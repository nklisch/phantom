use phantom_daemon::{engine, listener};

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use mio::Waker;

#[derive(Parser)]
#[command(name = "phantom-daemon", version)]
struct Args {
    /// Socket path
    #[arg(long)]
    socket: Option<String>,

    /// Run in foreground (don't daemonize)
    #[arg(long)]
    foreground: bool,

    /// Exit when this process is no longer running
    #[arg(long)]
    owner_pid: Option<u32>,

    /// Exit after this many idle seconds with no running sessions (0 disables)
    #[arg(long, default_value = "1800")]
    idle_timeout_secs: u64,
}

fn main() -> Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("phantom_daemon=info".parse().unwrap()),
        )
        .init();

    let socket_path = args
        .socket
        .map(PathBuf::from)
        .unwrap_or_else(default_socket_path);

    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();

    // We need to get the Waker from the engine, but the engine must be created
    // on the engine thread (because it contains !Send types).
    // Solution: create a oneshot channel to receive the Waker from the engine thread.
    let (waker_tx, waker_rx) = crossbeam_channel::bounded::<Arc<Waker>>(1);

    let engine_handle = std::thread::Builder::new()
        .name("phantom-engine".into())
        .spawn(move || match engine::Engine::new(cmd_rx) {
            Ok((mut engine, waker)) => {
                let _ = waker_tx.send(Arc::new(waker));
                if let Err(e) = engine.run() {
                    tracing::error!("Engine error: {e}");
                }
            }
            Err(e) => {
                tracing::error!("Failed to create engine: {e}");
            }
        })?;

    // Wait for the waker from the engine thread
    let waker = waker_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .map_err(|_| anyhow::anyhow!("Engine thread failed to start"))?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let lifecycle = listener::Lifecycle {
        owner_pid: args.owner_pid,
        idle_timeout: (args.idle_timeout_secs > 0)
            .then(|| Duration::from_secs(args.idle_timeout_secs)),
    };
    let build = phantom_daemon::build_identity();

    let listen_result = rt.block_on(async {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = listener::listen(
                &socket_path,
                cmd_tx.clone(),
                Arc::clone(&waker),
                build,
                lifecycle,
            ) => result,
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Received interrupt signal");
                Ok(())
            },
            _ = terminate.recv() => {
                tracing::info!("Received termination signal");
                Ok(())
            },
        }
    });

    let _ = cmd_tx.send(engine::EngineCommand::Shutdown);
    let _ = waker.wake();

    let _ = engine_handle.join();
    let _ = std::fs::remove_file(&socket_path);

    listen_result
}

fn default_socket_path() -> PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            PathBuf::from(home).join(".phantom")
        });
    runtime_dir.join("phantom.sock")
}
