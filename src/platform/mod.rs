//! Differences between operating systems. The Linux/macOS code lives in
//! `unix.rs` exactly as before; Windows has its own versions in
//! `windows.rs`. The rest of the editor only calls `platform::…`.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

/// Font files to try, in order of preference.
pub struct FontCandidates {
    pub ui: &'static [&'static str],
    pub mono: &'static [&'static str],
    /// Powerline / Nerd Font glyphs used by shell prompts.
    pub nerd: &'static [&'static str],
    /// Symbols for icons that the UI font may lack.
    pub symbols: &'static [&'static str],
}
