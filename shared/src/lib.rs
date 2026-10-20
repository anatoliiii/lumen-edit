//! Code shared between the editor and the remote agent: the wire protocol and
//! the filesystem/search operations that both sides execute locally.

pub mod fsops;
pub mod proto;
pub mod watch;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A path as the editor writes it.
#[cfg(unix)]
pub fn path_string(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

/// A path as the editor writes it: forward slashes, no `\\?\` prefix.
#[cfg(windows)]
pub fn path_string(path: &std::path::Path) -> String {
    path.to_string_lossy().trim_start_matches(r"\\?\").replace('\\', "/")
}
