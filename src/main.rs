//! zeron-pet entry point: CLI parse, compositor capability check, then the
//! single-instance desktop application (GTK layer-shell on unix, Win32
//! layered windows on Windows).

use std::time::Duration;
#[cfg(windows)]
use zeron_pet::desktop_win;
use zeron_pet::{cli, launcher};
#[cfg(unix)]
use zeron_pet::{desktop, layer_shell};

fn main() {
    env_logger::init();
    // args_os + lossy: env::args() panics on non-UTF8 argv.
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let cli = match cli::parse(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    if cli.help {
        print!("{}", cli::HELP);
        return;
    }

    // --launch is resolved before any desktop initialization: it probes
    // and, if needed, cold-starts a detached pet, then returns without
    // touching the display server itself.
    if cli.launch {
        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("zeron-pet --launch: cannot resolve current executable: {e}");
                std::process::exit(1);
            }
        };
        match launcher::launch(&exe, &cli.startup_args(), Duration::from_secs(8)) {
            Ok(status) => println!("{status}"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }

    // Honest compositor support check before claiming a layer surface.
    #[cfg(unix)]
    {
        if gtk4::init().is_err() {
            eprintln!("zeron-pet: cannot initialize GTK (no Wayland display?)");
            std::process::exit(3);
        }
        if !layer_shell::is_supported() {
            eprintln!(
                "zeron-pet: unsupported compositor — wlr layer-shell is required \
                 (Hyprland/wlroots). Not claiming X11 or non-layer-shell support."
            );
            std::process::exit(3);
        }
        std::process::exit(desktop::run(cli));
    }
    #[cfg(windows)]
    std::process::exit(desktop_win::run(cli));
}
