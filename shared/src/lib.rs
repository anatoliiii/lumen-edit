//! Code shared between the editor and the remote agent: the wire protocol and
//! the filesystem/search operations that both sides execute locally.

pub mod fsops;
pub mod proto;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
