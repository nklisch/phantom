pub mod backend;
pub mod engine;
pub mod handler;
pub mod input;
pub mod listener;
pub mod pty;
pub mod session;
pub mod wait;

pub fn build_identity() -> phantom_core::protocol::BuildIdentity {
    phantom_core::protocol::BuildIdentity {
        version: env!("CARGO_PKG_VERSION").to_string(),
        commit: env!("PHANTOM_BUILD_COMMIT").to_string(),
    }
}
