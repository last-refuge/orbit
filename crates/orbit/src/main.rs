//! Orbit entry point.
//!
//! M0 commands: `smoke` proves GPUI on the target host (ORB-001), `doctor`
//! records host capabilities and `fixture` opens owned test windows (ORB-002),
//! and `daemon` with `show`/`hide`/`toggle`/`status`/`quit` drives the overlay
//! lifecycle spike (ORB-003), `capture`/`preview` prove window previews
//! on fixture windows (ORB-004), `events` records the event stream for the
//! state store's replay tests (ORB-005), and the daemon's overlay is the
//! overview slice with `state`/`drive` for scripted checks (ORB-006).
//! `demo` arrives with M1.

mod assets;
mod backdrop;
mod capture;
mod corner;
mod daemon;
mod doctor;
mod engage;
mod events;
mod fixture;
mod prefs;
mod preview;
mod previews;
mod settings_window;
mod smoke;

use std::{path::PathBuf, process::ExitCode, time::Duration};

const USAGE: &str = "\
usage: orbit daemon [--test-hooks] [--high-contrast] [--opaque] [--glass|--no-glass] [--reduced-motion]
       orbit show|toggle [--output NAME]
       orbit hide|status|state|settings|quit
       orbit config list|path|get KEY|set KEY VALUE
       orbit assets install|remove [DIR]
       orbit drive key K[,K…] | type TEXT | compose TEXT | commit TEXT | click 0xADDR
                   | click-workspace NAME | click-search | drag 0xADDR NAME [follow] | a11y
       orbit smoke [--overlay] [--exit-after-ms N]
       orbit doctor [--json] [--record DIR]
       orbit fixture --name NAME [--exit-after-ms N]
       orbit capture [--json] [--record DIR] [--label NAME] [--repeat N]
       orbit preview [--interval-ms N] [--exit-after-ms N]
       orbit events [--duration-ms N] [--record FILE]
       orbit --version

  daemon             run the resident process that owns the overview
    --test-hooks     accept `drive` requests (scripts/slice-check.sh, review-check.sh)
    --high-contrast  opaque black canvas, brighter borders and text
    --opaque         the brand theme without translucency over the desktop
    --glass          the frosted-glass look (the default): a blurred capture of the
    --no-glass       desktop under the brand tint; --no-glass keeps the denser layers
    --reduced-motion no travel animation when the overview opens
  show, toggle       open (or toggle) the overview on NAME, default: the focused monitor
  hide, status, quit control the running daemon
  state              print the overview's state as JSON (no titles)
  settings           open the Orbit settings window
  config             read or change settings in $XDG_CONFIG_HOME/orbit/config.toml
                     (the running daemon applies changes within a second)
  assets             write (or remove) the desktop entry and icon-theme files under DIR
                     (default $XDG_DATA_HOME or ~/.local/share); the installer uses this
  drive              send keys, text, a click or a drag through the overview's input path,
                     or summarize its accessibility tree (no labels)
  smoke              open a plain GPUI window with the Orbit theme
    --overlay        open a fullscreen layer-shell overlay instead (Wayland only)
  doctor             report versions, monitors, workspaces and Wayland protocols
    --json           print the report as JSON
    --record DIR     also save the report and title-redacted Hyprland replies
  fixture            open an owned test window with app id dev.orbit.fixture.NAME
                     (NAME: lowercase letters, digits and dashes)
  capture            capture every open fixture window and check the pixels
    --record DIR     save report.json and fixture PNGs under DIR/LABEL
    --label NAME     scenario name for the record (default: run)
    --repeat N       frames per window for timing (default: 20)
  preview            show live fixture previews in a window, refreshed every N ms
  events             print Hyprland events for N ms (default 10000), titles redacted,
                     and check that each line parses; --record saves the lines
  --exit-after-ms N  quit by itself after N milliseconds (for scripted checks)

Escape closes the overview, smoke and fixture windows. Window titles are never printed or recorded.";

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse(&args) {
        Ok(Command::Version) => {
            println!("orbit {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Command::Smoke(options)) => {
            smoke::run(options);
            ExitCode::SUCCESS
        }
        Ok(Command::Daemon(options)) => match daemon::run(options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("orbit daemon: {error:#}");
                ExitCode::FAILURE
            }
        },
        Ok(Command::Assets { install, dir }) => {
            let result = if install {
                assets::install(&dir)
            } else {
                assets::remove(&dir)
            };
            match result {
                Ok(paths) => {
                    for path in paths {
                        println!("{}", path.display());
                    }
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("orbit assets: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Ok(Command::Config(action)) => match config_command(action) {
            Ok(text) => {
                if !text.is_empty() {
                    println!("{text}");
                }
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("orbit config: {error}");
                ExitCode::FAILURE
            }
        },
        Ok(Command::Control(request)) => match daemon::send(&request) {
            Ok(reply) => {
                println!("{reply}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("orbit: {error:#}");
                ExitCode::FAILURE
            }
        },
        Ok(Command::Fixture(options)) => {
            fixture::run(options);
            ExitCode::SUCCESS
        }
        Ok(Command::Capture(options)) => match capture::run(options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("orbit capture: {error:#}");
                ExitCode::FAILURE
            }
        },
        Ok(Command::Events(options)) => match events::run(options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("orbit events: {error:#}");
                ExitCode::FAILURE
            }
        },
        Ok(Command::Preview(options)) => {
            preview::run(options);
            ExitCode::SUCCESS
        }
        Ok(Command::Doctor(options)) => match doctor::run(options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("orbit doctor: {error:#}");
                ExitCode::FAILURE
            }
        },
        Err(message) => {
            if !message.is_empty() {
                eprintln!("orbit: {message}\n");
            }
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

enum Command {
    Version,
    Config(ConfigAction),
    Assets { install: bool, dir: PathBuf },
    Daemon(daemon::Options),
    Control(daemon::Request),
    Smoke(smoke::Options),
    Doctor(doctor::Options),
    Fixture(fixture::Options),
    Capture(capture::Options),
    Preview(preview::Options),
    Events(events::Options),
}

#[derive(Debug, PartialEq, Eq)]
enum ConfigAction {
    List,
    Path,
    Get(String),
    Set(String, String),
}

/// `orbit config …`: the file is the source of truth; the daemon notices
/// changes by itself.
fn config_command(action: ConfigAction) -> Result<String, String> {
    use orbit_core::Settings;
    let path = Settings::path().ok_or("no config directory (HOME is unset)")?;
    let settings = if path.exists() {
        Settings::load(&path)?
    } else {
        // First contact: the shortcut nothing else uses, as the daemon
        // would pick.
        Settings {
            shortcut: daemon::default_shortcut(),
            ..Settings::default()
        }
    };
    match action {
        ConfigAction::Path => Ok(path.display().to_string()),
        ConfigAction::List => Ok(settings
            .entries()
            .into_iter()
            .map(|(k, v)| format!("{k} = {v}"))
            .collect::<Vec<_>>()
            .join("\n")),
        ConfigAction::Get(key) => settings.get(&key),
        ConfigAction::Set(key, value) => {
            let updated = settings.set(&key, &value)?;
            updated.save(&path)?;
            updated.get(&key)
        }
    }
}

fn parse(args: &[String]) -> Result<Command, String> {
    let mut args = args.iter().map(String::as_str);
    let exit_after = |value: Option<&str>| {
        value
            .and_then(|value| value.parse().ok())
            .map(Duration::from_millis)
            .ok_or("--exit-after-ms needs a number of milliseconds")
    };
    match args.next() {
        Some("--version") => Ok(Command::Version),
        Some("assets") => {
            let rest: Vec<&str> = args.collect();
            let (install, dir) = match rest.as_slice() {
                ["install"] => (true, None),
                ["remove"] => (false, None),
                ["install", dir] => (true, Some(*dir)),
                ["remove", dir] => (false, Some(*dir)),
                _ => return Err("usage: orbit assets install|remove [DIR]".into()),
            };
            let dir = match dir {
                Some(dir) => PathBuf::from(dir),
                None => assets::default_dir().ok_or("no data directory (HOME is unset)")?,
            };
            Ok(Command::Assets { install, dir })
        }
        Some("daemon") => {
            let mut options = daemon::Options::default();
            for arg in args {
                match arg {
                    "--test-hooks" => options.test_hooks = true,
                    "--high-contrast" => options.high_contrast = true,
                    "--reduced-motion" => options.reduced_motion = true,
                    "--opaque" => options.opaque = true,
                    "--glass" => options.glass = Some(true),
                    "--no-glass" => options.glass = Some(false),
                    other => return Err(format!("unknown daemon option `{other}`")),
                }
            }
            Ok(Command::Daemon(options))
        }
        Some(command @ ("show" | "toggle")) => {
            let output = match (args.next(), args.next(), args.next()) {
                (None, _, _) => None,
                (Some("--output"), Some(name), None) => Some(name.to_owned()),
                _ => return Err(format!("usage: orbit {command} [--output NAME]")),
            };
            Ok(Command::Control(if command == "show" {
                daemon::Request::Show { output }
            } else {
                daemon::Request::Toggle { output }
            }))
        }
        Some("drive") => {
            let rest: Vec<&str> = args.collect();
            if rest.is_empty() {
                return Err("usage: orbit drive key K[,K…] | type TEXT | compose TEXT | commit TEXT | click 0xADDR | click-workspace NAME | click-search | drag 0xADDR NAME [follow] | a11y".into());
            }
            Ok(Command::Control(daemon::Request::Drive(rest.join(" "))))
        }
        Some("config") => {
            let rest: Vec<&str> = args.collect();
            match rest.as_slice() {
                ["list"] => Ok(Command::Config(ConfigAction::List)),
                ["path"] => Ok(Command::Config(ConfigAction::Path)),
                ["get", key] => Ok(Command::Config(ConfigAction::Get((*key).to_owned()))),
                ["set", key, value @ ..] if !value.is_empty() => Ok(Command::Config(
                    ConfigAction::Set((*key).to_owned(), value.join(" ")),
                )),
                _ => Err("usage: orbit config list | path | get KEY | set KEY VALUE".into()),
            }
        }
        Some(command @ ("hide" | "status" | "quit" | "state" | "settings")) => {
            if let Some(other) = args.next() {
                return Err(format!("`{command}` takes no options, got `{other}`"));
            }
            Ok(Command::Control(
                daemon::Request::parse(command).map_err(|error| error.to_string())?,
            ))
        }
        Some("smoke") => {
            let mut options = smoke::Options::default();
            while let Some(arg) = args.next() {
                match arg {
                    "--overlay" => options.overlay = true,
                    "--exit-after-ms" => options.exit_after = Some(exit_after(args.next())?),
                    other => return Err(format!("unknown smoke option `{other}`")),
                }
            }
            Ok(Command::Smoke(options))
        }
        Some("doctor") => {
            let mut options = doctor::Options::default();
            while let Some(arg) = args.next() {
                match arg {
                    "--json" => options.json = true,
                    "--record" => {
                        let dir = args.next().ok_or("--record needs a directory")?;
                        options.record = Some(PathBuf::from(dir));
                    }
                    other => return Err(format!("unknown doctor option `{other}`")),
                }
            }
            Ok(Command::Doctor(options))
        }
        Some("fixture") => {
            let mut name = None;
            let mut exit = None;
            while let Some(arg) = args.next() {
                match arg {
                    "--name" => name = Some(args.next().ok_or("--name needs a value")?),
                    "--exit-after-ms" => exit = Some(exit_after(args.next())?),
                    other => return Err(format!("unknown fixture option `{other}`")),
                }
            }
            let name = name.ok_or("fixture needs --name")?;
            if !fixture::valid_name(name) {
                return Err(format!(
                    "fixture name `{name}` must be 1-32 lowercase letters, digits or dashes"
                ));
            }
            Ok(Command::Fixture(fixture::Options {
                name: name.to_owned(),
                exit_after: exit,
            }))
        }
        Some("capture") => {
            let mut options = capture::Options::default();
            while let Some(arg) = args.next() {
                match arg {
                    "--json" => options.json = true,
                    "--record" => {
                        let dir = args.next().ok_or("--record needs a directory")?;
                        options.record = Some(PathBuf::from(dir));
                    }
                    "--label" => {
                        let label = args.next().ok_or("--label needs a name")?;
                        if !fixture::valid_name(label) {
                            return Err(format!(
                                "label `{label}` must be 1-32 lowercase letters, digits or dashes"
                            ));
                        }
                        options.label = label.to_owned();
                    }
                    "--repeat" => {
                        options.repeat = args
                            .next()
                            .and_then(|n| n.parse().ok())
                            .filter(|n| (1..=1000).contains(n))
                            .ok_or("--repeat needs a number from 1 to 1000")?;
                    }
                    other => return Err(format!("unknown capture option `{other}`")),
                }
            }
            Ok(Command::Capture(options))
        }
        Some("preview") => {
            let mut options = preview::Options::default();
            while let Some(arg) = args.next() {
                match arg {
                    "--interval-ms" => {
                        options.interval = exit_after(args.next())
                            .map_err(|_| "--interval-ms needs a number of milliseconds")?
                    }
                    "--exit-after-ms" => options.exit_after = Some(exit_after(args.next())?),
                    other => return Err(format!("unknown preview option `{other}`")),
                }
            }
            Ok(Command::Preview(options))
        }
        Some("events") => {
            let mut options = events::Options::default();
            while let Some(arg) = args.next() {
                match arg {
                    "--duration-ms" => {
                        options.duration = exit_after(args.next())
                            .map_err(|_| "--duration-ms needs a number of milliseconds")?
                    }
                    "--record" => {
                        let file = args.next().ok_or("--record needs a file")?;
                        options.record = Some(PathBuf::from(file));
                    }
                    other => return Err(format!("unknown events option `{other}`")),
                }
            }
            Ok(Command::Events(options))
        }
        Some(other) => Err(format!("unknown command `{other}`")),
        None => Err(String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn smoke_defaults_to_a_plain_window() {
        let Ok(Command::Smoke(options)) = parse(&args(&["smoke"])) else {
            panic!("expected smoke");
        };
        assert!(!options.overlay);
        assert_eq!(options.exit_after, None);
    }

    #[test]
    fn smoke_accepts_overlay_and_exit_delay() {
        let Ok(Command::Smoke(options)) =
            parse(&args(&["smoke", "--exit-after-ms", "250", "--overlay"]))
        else {
            panic!("expected smoke");
        };
        assert!(options.overlay);
        assert_eq!(options.exit_after, Some(Duration::from_millis(250)));
    }

    #[test]
    fn rejects_unknown_input() {
        assert!(parse(&args(&[])).is_err());
        assert!(parse(&args(&["overview"])).is_err());
        assert!(parse(&args(&["smoke", "--exit-after-ms"])).is_err());
        assert!(parse(&args(&["smoke", "--exit-after-ms", "soon"])).is_err());
        assert!(parse(&args(&["smoke", "--fullscreen"])).is_err());
    }

    #[test]
    fn doctor_options() {
        let Ok(Command::Doctor(options)) = parse(&args(&["doctor", "--record", "out", "--json"]))
        else {
            panic!("expected doctor");
        };
        assert!(options.json);
        assert_eq!(options.record, Some(PathBuf::from("out")));
        assert!(parse(&args(&["doctor", "--record"])).is_err());
    }

    #[test]
    fn control_commands() {
        let Ok(Command::Control(request)) = parse(&args(&["toggle", "--output", "DP-1"])) else {
            panic!("expected control");
        };
        assert_eq!(
            request,
            daemon::Request::Toggle {
                output: Some("DP-1".into())
            }
        );
        assert!(matches!(
            parse(&args(&["hide"])),
            Ok(Command::Control(daemon::Request::Hide))
        ));
        assert!(matches!(parse(&args(&["daemon"])), Ok(Command::Daemon(_))));
        let Ok(Command::Daemon(options)) = parse(&args(&["daemon", "--glass", "--test-hooks"]))
        else {
            panic!("expected daemon");
        };
        assert_eq!(options.glass, Some(true));
        assert!(options.test_hooks);
        let Ok(Command::Daemon(options)) = parse(&args(&["daemon", "--no-glass"])) else {
            panic!("expected daemon");
        };
        assert_eq!(options.glass, Some(false));
        assert!(parse(&args(&["show", "DP-1"])).is_err());
        assert!(matches!(
            parse(&args(&["settings"])),
            Ok(Command::Control(daemon::Request::Settings))
        ));
        assert_eq!(
            parse(&args(&["config", "set", "hot_corner.corner", "top-left"])).ok().map(|c| matches!(c, Command::Config(ConfigAction::Set(k, v)) if k == "hot_corner.corner" && v == "top-left")),
            Some(true)
        );
        assert!(parse(&args(&["config", "frob"])).is_err());
        assert!(parse(&args(&["hide", "--output", "DP-1"])).is_err());
    }

    #[test]
    fn fixture_requires_a_safe_name() {
        let Ok(Command::Fixture(options)) = parse(&args(&["fixture", "--name", "alpha"])) else {
            panic!("expected fixture");
        };
        assert_eq!(options.name, "alpha");
        assert!(parse(&args(&["fixture"])).is_err());
        assert!(parse(&args(&["fixture", "--name", "Bad Name"])).is_err());
    }
}
