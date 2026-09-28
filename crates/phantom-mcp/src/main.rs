//! phantom-mcp — MCP server exposing phantom for AI agents over stdio.

use anyhow::Result;
use phantom_mcp::{observer, server::PhantomMcpServer};
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    // Logs go to stderr — stdout is reserved for MCP protocol traffic.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("phantom_mcp=info,phantom_daemon=warn")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    tracing::info!("starting phantom-mcp");
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;

    // Build the server (this spawns the engine thread).
    let server = PhantomMcpServer::new()?;

    // Bind the observer socket and spawn its accept loop. We do this before
    // serving stdio so the socket file is guaranteed to exist by the time the
    // first tool call lands. The path can be overridden via PHANTOM_MCP_SOCKET.
    let socket_path = observer::resolve_socket_path();
    let (cmd_tx, waker) = server.engine_handle();
    let shutdown_tx = cmd_tx.clone();
    let shutdown_waker = waker.clone();
    let _observer = observer::serve(&socket_path, cmd_tx, waker).await?;
    let server = server.with_observer_socket(socket_path.clone());

    let service = tokio::select! {
        result = server.serve(stdio()) => match result {
            Ok(service) => service,
            Err(e) if e.to_string().contains("connection closed") => return Ok(()),
            Err(e) => {
                tracing::error!("serving error: {e}");
                return Err(e.into());
            }
        },
        _ = interrupt.recv() => {
            tracing::info!("received interrupt signal");
            exit_on_signal(&socket_path, &shutdown_tx, &shutdown_waker);
        },
        _ = terminate.recv() => {
            tracing::info!("received termination signal");
            exit_on_signal(&socket_path, &shutdown_tx, &shutdown_waker);
        },
    };

    tokio::select! {
        result = service.waiting() => {
            let _ = result?;
        },
        _ = interrupt.recv() => {
            tracing::info!("received interrupt signal");
            exit_on_signal(&socket_path, &shutdown_tx, &shutdown_waker);
        },
        _ = terminate.recv() => {
            tracing::info!("received termination signal");
            exit_on_signal(&socket_path, &shutdown_tx, &shutdown_waker);
        },
    }
    stop_engine(&shutdown_tx, &shutdown_waker);
    Ok(())
}

fn exit_on_signal(
    socket_path: &std::path::Path,
    cmd_tx: &crossbeam_channel::Sender<phantom_daemon::engine::EngineCommand>,
    waker: &mio::Waker,
) -> ! {
    stop_engine(cmd_tx, waker);
    let _ = std::fs::remove_file(socket_path);
    // Tokio's stdio transport owns a blocking stdin read. It cannot be
    // cancelled while the parent keeps stdin open, so terminate after the
    // engine and socket have been cleaned up explicitly.
    std::process::exit(0)
}

fn stop_engine(
    cmd_tx: &crossbeam_channel::Sender<phantom_daemon::engine::EngineCommand>,
    waker: &mio::Waker,
) {
    let _ = cmd_tx.send(phantom_daemon::engine::EngineCommand::Shutdown);
    let _ = waker.wake();
}
