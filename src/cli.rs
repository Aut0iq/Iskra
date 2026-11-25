//! Command-line surface. The first launch configures the pet; when an
//! instance is already running, control flags are forwarded to it through
//! the GTK single-instance mechanism, which is what makes click-through
//! recoverable without clicking the pet.

use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct Cli {
    pub assets: Option<PathBuf>,
    pub scale: u32,
    pub monitor: Option<String>,
    pub demo: Option<String>,
    pub cycle_demo: bool,
    /// Display-only support pose override (verification/recovery control).
    pub pose: Option<String>,
    // Control commands (also valid on a running instance).
    pub status: bool,
    pub quit: bool,
    pub pause: bool,
    pub resume: bool,
    pub live: bool,
    pub click_through: Option<bool>,
    pub help: bool,
    /// Launch-only command (used by Zeron): ensure the desktop pet is
    /// running, print its status, return. Handled before GTK init.
    pub launch: bool,
}

/// Demo poses accepted by --demo. `blush`/`dragged` are display-only
/// reaction controls forwarded to a running instance, never a launch.
pub const DEMO_NAMES: [&str; 11] = [
    "idle",
    "dance_back",
    "focus",
    "joy",
    "anger",
    "sadness",
    "question",
    "subdued",
    "disconnected",
    "blush",
    "dragged",
];

pub const HELP: &str = "\
zeron-pet — transparent desktop companion reporting Zeron chat status.

Startup options (first instance only):
  --assets PATH        asset directory containing manifest.json + frames/
  --scale INTEGER      integer sprite scale (default 2; 96x128 frames)
  --monitor CONNECTOR  pin the pet to an output, e.g. eDP-1
  --demo EMOTION       display-only demo pose: idle|dance_back|focus|joy|anger|
                       sadness|question|subdued|disconnected
                       (never used as live status)
  --cycle-demo         cycle through all demo poses
  --pose POSE          display-only support pose:
                       standing|floor|wall-left|wall-right
                       (snaps contact via the normal behavior path;
                       forwarded to a running instance, never launches)

Launch (used by Zeron; returns once the pet is confirmed up):
  --launch             ensure the desktop pet is running and print its
                       status; cold-starts a detached instance when absent,
                       no-op when already running. Only --assets/--scale/
                       --monitor may accompany it.

Control commands (forwarded to the running instance; they fail with
'not running' when no instance is up):
  --status             print a one-line status summary
  --pause              pause all animation playback
  --resume             resume animation playback (a manually pinned
                       position survives pause/resume)
  --live               leave demo mode, return to real statuses
  --click-through on|off  make the surface ignore pointer input; recover
                       with `--click-through off` from any terminal
  --quit               shut the pet down
  --help               this text
";

pub fn parse(args: &[String]) -> Result<Cli, String> {
    let mut cli = Cli {
        scale: 2,
        ..Cli::default()
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--assets" => {
                i += 1;
                let v = args.get(i).ok_or("--assets requires PATH")?;
                cli.assets = Some(PathBuf::from(v));
            }
            "--scale" => {
                i += 1;
                let v = args.get(i).ok_or("--scale requires INTEGER")?;
                cli.scale = v
                    .parse::<u32>()
                    .map_err(|_| format!("--scale: '{v}' is not an integer"))?;
                if cli.scale == 0 || cli.scale > 8 {
                    return Err("--scale must be 1..=8".to_string());
                }
            }
            "--monitor" => {
                i += 1;
                cli.monitor = Some(args.get(i).ok_or("--monitor requires CONNECTOR")?.clone());
            }
            "--demo" => {
                i += 1;
                let v = args.get(i).ok_or("--demo requires EMOTION")?;
                if !DEMO_NAMES.contains(&v.as_str()) {
                    return Err(format!(
                        "--demo: unknown emotion '{v}' (one of {})",
                        DEMO_NAMES.join("|")
                    ));
                }
                cli.demo = Some(v.clone());
            }
            "--cycle-demo" => cli.cycle_demo = true,
            "--pose" => {
                i += 1;
                let v = args.get(i).ok_or("--pose requires POSE")?;
                if crate::behavior::Placement::parse(v).is_none() {
                    return Err(format!(
                        "--pose: unknown pose '{v}' (standing|floor|wall-left|wall-right)"
                    ));
                }
                cli.pose = Some(v.clone());
            }
            "--status" => cli.status = true,
            "--quit" => cli.quit = true,
            "--pause" => cli.pause = true,
            "--resume" => cli.resume = true,
            "--live" => cli.live = true,
            "--click-through" => {
                i += 1;
                match args.get(i).map(String::as_str) {
                    Some("on") => cli.click_through = Some(true),
                    Some("off") => cli.click_through = Some(false),
                    other => {
                        return Err(format!(
                            "--click-through requires 'on' or 'off', got {other:?}"
                        ))
                    }
                }
            }
            "--launch" => cli.launch = true,
            "--help" | "-h" => cli.help = true,
            other => return Err(format!("unknown option '{other}' (try --help)")),
        }
        i += 1;
    }
    if cli.pause && cli.resume {
        return Err("--pause and --resume are mutually exclusive".to_string());
    }
    if cli.live && (cli.demo.is_some() || cli.cycle_demo) {
        return Err("--live conflicts with --demo/--cycle-demo".to_string());
    }
    if cli.demo.is_some() && cli.cycle_demo {
        return Err("--demo and --cycle-demo are mutually exclusive".to_string());
    }
    if cli.pose.is_some() && (cli.demo.is_some() || cli.cycle_demo) {
        return Err("--pose conflicts with --demo/--cycle-demo".to_string());
    }
    if cli.launch
        && (cli.status
            || cli.quit
            || cli.pause
            || cli.resume
            || cli.live
            || cli.click_through.is_some()
            || cli.pose.is_some()
            || cli.help
            || cli.demo.is_some()
            || cli.cycle_demo)
    {
        return Err("--launch is launch-only: it cannot be combined with \
             status/quit/pause/resume/live/click-through/pose/help/demo flags"
            .to_string());
    }
    Ok(cli)
}

impl Cli {
    /// Normalized startup-only argv for a `--launch` cold start. Rebuilt
    /// from parsed fields rather than copied from raw argv, so values with
    /// spaces — even a literal `--launch` used as a connector name — reach
    /// the child verbatim.
    pub fn startup_args(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(a) = &self.assets {
            v.push("--assets".to_string());
            v.push(a.to_string_lossy().into_owned());
        }
        if self.scale != 2 {
            v.push("--scale".to_string());
            v.push(self.scale.to_string());
        }
        if let Some(m) = &self.monitor {
            v.push("--monitor".to_string());
            v.push(m.clone());
        }
        v
    }
}

/// True when the invocation carries only remote-control flags and no startup
/// configuration. `--pose` and the reaction demos `blush`/`dragged` are
/// display-only controls: they never launch a new pet.
pub fn is_control_only(cli: &Cli) -> bool {
    let reaction_demo = matches!(cli.demo.as_deref(), Some("blush") | Some("dragged"));
    !cli.launch
        && cli.assets.is_none()
        && cli.monitor.is_none()
        && (cli.demo.is_none() || reaction_demo)
        && !cli.cycle_demo
        && cli.scale == 2
        && (cli.status
            || cli.quit
            || cli.pause
            || cli.resume
            || cli.live
            || cli.click_through.is_some()
            || cli.pose.is_some()
            || reaction_demo)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_startup_flags() {
        let c = parse(&argv(&[
            "--assets",
            "/a",
            "--scale",
            "3",
            "--monitor",
            "eDP-1",
            "--demo",
            "joy",
        ]))
        .unwrap();
        assert_eq!(c.scale, 3);
        assert_eq!(c.monitor.as_deref(), Some("eDP-1"));
        assert_eq!(c.demo.as_deref(), Some("joy"));
        assert!(!c.cycle_demo);
        assert!(parse(&argv(&["--cycle-demo"])).unwrap().cycle_demo);
    }

    #[test]
    fn rejects_bad_values() {
        assert!(parse(&argv(&["--scale", "x"])).is_err());
        assert!(parse(&argv(&["--scale", "0"])).is_err());
        assert!(parse(&argv(&["--click-through", "yes"])).is_err());
        assert!(parse(&argv(&["--bogus"])).is_err());
        assert!(parse(&argv(&["--demo", "dance_back"])).is_ok());
        assert!(parse(&argv(&["--demo", "walk"])).is_err());
        assert!(parse(&argv(&["--demo", "blush"])).is_ok());
        assert!(parse(&argv(&["--demo", "dragged"])).is_ok());
        assert!(parse(&argv(&["--pose", "floor"])).is_ok());
        assert!(parse(&argv(&["--pose", "wall-left"])).is_ok());
        assert!(parse(&argv(&["--pose", "bogus"])).is_err());
        assert!(parse(&argv(&["--pause", "--resume"])).is_err());
    }

    #[test]
    fn demo_and_live_validation() {
        assert!(parse(&argv(&["--demo", "bogus"])).is_err());
        assert!(parse(&argv(&["--demo", "joy"])).is_ok());
        assert!(parse(&argv(&["--demo", "subdued"])).is_ok());
        assert!(parse(&argv(&["--live"])).is_ok());
        assert!(parse(&argv(&["--live", "--demo", "joy"])).is_err());
        assert!(parse(&argv(&["--live", "--cycle-demo"])).is_err());
        assert!(parse(&argv(&["--demo", "joy", "--cycle-demo"])).is_err());
        assert!(parse(&argv(&["--pose", "floor", "--demo", "joy"])).is_err());
    }

    #[test]
    fn detects_control_only() {
        assert!(is_control_only(&parse(&argv(&["--status"])).unwrap()));
        assert!(is_control_only(
            &parse(&argv(&["--click-through", "off"])).unwrap()
        ));
        assert!(is_control_only(&parse(&argv(&["--live"])).unwrap()));
        assert!(!is_control_only(
            &parse(&argv(&["--monitor", "eDP-1"])).unwrap()
        ));
        assert!(!is_control_only(&Cli::default()));
        // --pose and reaction demos are controls: never launch a pet.
        assert!(is_control_only(
            &parse(&argv(&["--pose", "floor"])).unwrap()
        ));
        assert!(is_control_only(
            &parse(&argv(&["--demo", "blush"])).unwrap()
        ));
        assert!(is_control_only(
            &parse(&argv(&["--demo", "dragged"])).unwrap()
        ));
        assert!(!is_control_only(&parse(&argv(&["--demo", "joy"])).unwrap()));
        assert!(!is_control_only(
            &parse(&argv(&["--pose", "floor", "--monitor", "eDP-1"])).unwrap()
        ));
    }

    #[test]
    fn launch_accepts_startup_options_only() {
        assert!(parse(&argv(&["--launch"])).unwrap().launch);
        let c = parse(&argv(&[
            "--launch",
            "--assets",
            "/a b",
            "--scale",
            "3",
            "--monitor",
            "eDP-1",
        ]))
        .unwrap();
        assert!(c.launch);
        assert_eq!(c.scale, 3);
        assert_eq!(c.monitor.as_deref(), Some("eDP-1"));
        // Every control/demo/help flag is rejected beside --launch.
        for extra in [
            &["--status"][..],
            &["--quit"][..],
            &["--pause"][..],
            &["--resume"][..],
            &["--live"][..],
            &["--click-through", "on"][..],
            &["--click-through", "off"][..],
            &["--pose", "floor"][..],
            &["--help"][..],
            &["-h"][..],
            &["--demo", "joy"][..],
            &["--demo", "blush"][..],
            &["--cycle-demo"][..],
        ] {
            let mut a = vec!["--launch"];
            a.extend_from_slice(extra);
            assert!(parse(&argv(&a)).is_err(), "--launch must reject {extra:?}");
        }
        // Launch is never a control-only invocation.
        assert!(!is_control_only(&parse(&argv(&["--launch"])).unwrap()));
    }

    #[test]
    fn startup_args_preserve_values_verbatim() {
        let c = parse(&argv(&[
            "--launch",
            "--assets",
            "/dir with spaces/assets",
            "--scale",
            "4",
            "--monitor",
            "eDP-1 (built-in)",
        ]))
        .unwrap();
        assert_eq!(
            c.startup_args(),
            vec![
                "--assets",
                "/dir with spaces/assets",
                "--scale",
                "4",
                "--monitor",
                "eDP-1 (built-in)"
            ]
        );
        // A literal "--launch" consumed as a value stays a value.
        let c = parse(&argv(&["--launch", "--monitor", "--launch"])).unwrap();
        assert_eq!(c.startup_args(), vec!["--monitor", "--launch"]);
        // Defaults produce no argv at all.
        assert!(parse(&argv(&["--launch"]))
            .unwrap()
            .startup_args()
            .is_empty());
    }
}
