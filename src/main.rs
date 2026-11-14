//! zeron-pet entry point: CLI parse, compositor capability check, then the
//! single-instance GTK application.

use std::time::Duration;
use zeron_pet::{cli, desktop, launcher, layer_shell};

fn main() {
    env_logger::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
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

    // --launch is resolved before any GTK initialization: it probes and, if
    // needed, cold-starts a detached pet, then returns without touching the
    // display server itself.
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
