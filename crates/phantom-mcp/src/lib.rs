//! phantom-mcp — MCP server exposing phantom for AI agents.
//!
//! This crate's primary artifact is the `phantom-mcp` binary (see `src/main.rs`),
//! which speaks the Model Context Protocol over stdio. The library surface is
//! exposed so integration tests can drive the server without going through
//! the wire protocol.

pub mod observer;
pub mod server;
pub mod tmux;

pub fn build_identity() -> phantom_core::protocol::BuildIdentity {
    phantom_core::protocol::BuildIdentity {
        version: env!("CARGO_PKG_VERSION").to_string(),
        commit: env!("PHANTOM_BUILD_COMMIT").to_string(),
    }
}
