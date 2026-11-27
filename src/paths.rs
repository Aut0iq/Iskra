//! Tiny cross-platform path helpers.

use std::path::PathBuf;

/// User home directory: `$HOME` first (set by some Windows environments
/// too), then `%USERPROFILE%`. Empty values count as unset.
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| {
            if cfg!(windows) {
                std::env::var_os("USERPROFILE")
            } else {
                None
            }
        })
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}
