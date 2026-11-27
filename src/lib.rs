pub mod assets;
pub mod behavior;
pub mod cli;
pub mod codex_limits;
pub mod codex_probe;
#[cfg(unix)]
pub mod desktop;
pub mod desktop_shared;
#[cfg(windows)]
pub mod desktop_win;
pub mod hyprland;
pub mod launcher;
#[cfg(unix)]
pub mod layer_shell;
pub mod model;
pub mod motion;
pub mod paths;
pub mod prep;
pub mod timefmt;
pub mod usage;
#[cfg(windows)]
pub mod winmon;
pub mod zeron;
