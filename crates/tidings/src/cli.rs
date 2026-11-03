//! Command-line interface definitions.

use clap::{Args, Parser, Subcommand, ValueEnum};
use tidings::ipc::proto::{DndChange, Request};
use tidings_core::gesture::{GestureEvent, GestureKind};

/// macOS-style Notification Centre for Hyprland.
#[derive(Debug, Parser)]
#[command(name = "tidings", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the notification daemon (the default).
    Daemon(DaemonArgs),
    /// Control the running daemon over its IPC socket.
    ///
    /// Exit codes: 0 ok, 1 error, 2 daemon not running.
    Ctl {
        #[command(subcommand)]
        command: CtlCommand,
    },
    /// Inspect the config file (config.toml).
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// List, show and check themes.
    Theme {
        #[command(subcommand)]
        command: ThemeCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Check a config file for errors (default: the one tidings would load).
    ///
    /// Exit codes: 0 valid (or absent, so the defaults apply), 1 invalid.
    Check { path: Option<std::path::PathBuf> },
    /// Print a commented config file with every setting at its default.
    Default,
    /// Print where tidings looks for its config file.
    Path,
}

#[derive(Debug, Subcommand)]
pub enum ThemeCommand {
    /// List the themes: built-in ones and files in the themes folder.
    List,
    /// Print a theme's file (to copy into the themes folder and edit).
    Show { name: String },
    /// Check a theme for errors: a name, or a path to a .toml file
    /// (default: the theme the config chooses).
    ///
    /// Exit codes: 0 valid, 1 invalid or not found.
    Check { theme: Option<String> },
    /// Print the folder tidings reads theme files from.
    Path,
}

#[derive(Debug, Default, Args)]
pub struct DaemonArgs {
    /// Don't read the trackpad (no edge-swipe gesture).
    #[arg(long)]
    pub no_gestures: bool,
}

#[derive(Debug, Subcommand)]
pub enum CtlCommand {
    /// Open the panel if closed, close it if open.
    Toggle,
    /// Open the panel.
    Open,
    /// Close the panel.
    Close,
    /// Dismiss every notification.
    ClearAll,
    /// Show or change Do Not Disturb.
    Dnd { state: Option<DndArg> },
    /// Print the number of notifications.
    Count,
    /// Print whether the panel is open ("true"/"false").
    IsOpen,
    /// Inject a gesture event, as if from the trackpad (for tests).
    Gesture {
        #[command(subcommand)]
        event: GestureCommand,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DndArg {
    On,
    Off,
    Toggle,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum KindArg {
    Open,
    Close,
}

#[derive(Debug, Subcommand)]
pub enum GestureCommand {
    /// Start a gesture.
    Begin { kind: KindArg },
    /// Progress as a fraction of pad width (may be negative).
    Update {
        #[arg(allow_negative_numbers = true)]
        progress: f32,
    },
    /// Fingers lifted with this progress and velocity (pad widths/s).
    End {
        #[arg(allow_negative_numbers = true)]
        progress: f32,
        #[arg(allow_negative_numbers = true)]
        velocity: f32,
    },
    /// Abort the gesture.
    Cancel,
}

impl From<CtlCommand> for Request {
    fn from(cmd: CtlCommand) -> Self {
        match cmd {
            CtlCommand::Toggle => Self::Toggle,
            CtlCommand::Open => Self::Open,
            CtlCommand::Close => Self::Close,
            CtlCommand::ClearAll => Self::ClearAll,
            CtlCommand::Dnd { state } => Self::Dnd(state.map(|s| match s {
                DndArg::On => DndChange::On,
                DndArg::Off => DndChange::Off,
                DndArg::Toggle => DndChange::Toggle,
            })),
            CtlCommand::Count => Self::Count,
            CtlCommand::IsOpen => Self::IsOpen,
            CtlCommand::Gesture { event } => Self::Gesture(match event {
                GestureCommand::Begin {
                    kind: KindArg::Open,
                } => GestureEvent::Begin(GestureKind::Open),
                GestureCommand::Begin {
                    kind: KindArg::Close,
                } => GestureEvent::Begin(GestureKind::Close),
                GestureCommand::Update { progress } => GestureEvent::Update { progress },
                GestureCommand::End { progress, velocity } => {
                    GestureEvent::End { progress, velocity }
                }
                GestureCommand::Cancel => GestureEvent::Cancel,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn ctl(args: &[&str]) -> Request {
        let cli = Cli::try_parse_from(["tidings", "ctl"].iter().chain(args)).unwrap();
        match cli.command {
            Some(Command::Ctl { command }) => command.into(),
            other => panic!("expected ctl, got {other:?}"),
        }
    }

    #[test]
    fn cli_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn default_is_daemon() {
        assert!(Cli::try_parse_from(["tidings"]).unwrap().command.is_none());
        let cli = Cli::try_parse_from(["tidings", "daemon", "--no-gestures"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Daemon(DaemonArgs { no_gestures: true }))
        ));
    }

    #[test]
    fn ctl_commands_map_to_requests() {
        assert_eq!(ctl(&["toggle"]), Request::Toggle);
        assert_eq!(ctl(&["open"]), Request::Open);
        assert_eq!(ctl(&["close"]), Request::Close);
        assert_eq!(ctl(&["clear-all"]), Request::ClearAll);
        assert_eq!(ctl(&["count"]), Request::Count);
        assert_eq!(ctl(&["is-open"]), Request::IsOpen);
        assert_eq!(ctl(&["dnd"]), Request::Dnd(None));
        assert_eq!(ctl(&["dnd", "on"]), Request::Dnd(Some(DndChange::On)));
        assert_eq!(ctl(&["dnd", "off"]), Request::Dnd(Some(DndChange::Off)));
        assert_eq!(
            ctl(&["gesture", "begin", "open"]),
            Request::Gesture(GestureEvent::Begin(GestureKind::Open))
        );
        assert_eq!(
            ctl(&["dnd", "toggle"]),
            Request::Dnd(Some(DndChange::Toggle))
        );
        assert_eq!(
            ctl(&["gesture", "begin", "close"]),
            Request::Gesture(GestureEvent::Begin(GestureKind::Close))
        );
        assert_eq!(
            ctl(&["gesture", "update", "-0.2"]),
            Request::Gesture(GestureEvent::Update { progress: -0.2 })
        );
        assert_eq!(
            ctl(&["gesture", "end", "0.4", "-1"]),
            Request::Gesture(GestureEvent::End {
                progress: 0.4,
                velocity: -1.0
            })
        );
        assert_eq!(
            ctl(&["gesture", "cancel"]),
            Request::Gesture(GestureEvent::Cancel)
        );
        assert!(Cli::try_parse_from(["tidings", "ctl", "dnd", "maybe"]).is_err());
    }
}
