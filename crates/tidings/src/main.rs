//! `tidings`: run the daemon, control it with `tidings ctl ...`, or check
//! its config with `tidings config ...`.

mod cli;

use std::process::ExitCode;
use std::sync::Arc;
use std::sync::mpsc::Sender;

use anyhow::Context as _;
use tidings::clock::SystemClock;
use tidings::config_file::{self, LoadError};
use tidings::daemon::{Daemon, DaemonConfig};
use tidings::ipc;
use tidings::ipc::client::{self, ClientError};
use tidings::ipc::proto::{Request, Response};
use tidings::themes::{self, Origin};
use tidings_core::config::theme::DEFAULT_THEME;
use tidings_core::config::{AnimationConfig, TEMPLATE};
use tidings_core::look::Look;
use tidings_core::ui::{UiEvent, UiHandle};
use tracing_subscriber::EnvFilter;

use clap::Parser as _;

use crate::cli::{Cli, Command, ConfigCommand, DaemonArgs, ThemeCommand};

/// The only place the UI is created: starts the Wayland UI thread.
///
/// `Daemon::start` calls this only after the bus name and IPC socket are
/// acquired, so a refused start never shows any surface.
fn spawn_ui(
    events: Sender<UiEvent>,
    animation: AnimationConfig,
    look: &Look,
) -> Result<Box<dyn UiHandle>, Box<dyn std::error::Error + Send + Sync>> {
    Ok(tidings_ui::spawn(events, animation, look)?)
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            // Usage errors exit 1; exit code 2 means "daemon not running".
            let code = u8::from(e.use_stderr());
            let _ = e.print();
            return ExitCode::from(code);
        }
    };
    match cli
        .command
        .unwrap_or(Command::Daemon(DaemonArgs::default()))
    {
        Command::Ctl { command } => ctl(command.into()),
        Command::Config { command } => config(command),
        Command::Theme { command } => theme(command),
        Command::Daemon(args) => {
            init_logging();
            match run_daemon(&args) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("tidings: {e:#}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

/// `tidings config ...`: check, print the template, or print the path.
fn config(command: ConfigCommand) -> ExitCode {
    match command {
        ConfigCommand::Default => {
            print!("{TEMPLATE}");
            ExitCode::SUCCESS
        }
        ConfigCommand::Path => {
            let Some(path) = config_file::config_path() else {
                eprintln!("tidings: neither XDG_CONFIG_HOME nor HOME is set");
                return ExitCode::FAILURE;
            };
            println!("{}", path.display());
            ExitCode::SUCCESS
        }
        ConfigCommand::Check { path } => {
            let Some(path) = path.or_else(config_file::config_path) else {
                eprintln!("tidings: neither XDG_CONFIG_HOME nor HOME is set");
                return ExitCode::FAILURE;
            };
            match config_file::load(&path) {
                Ok(_) => {
                    println!("{}: ok", path.display());
                    ExitCode::SUCCESS
                }
                Err(LoadError::Missing(path)) => {
                    println!("{}: no such file; the defaults apply", path.display());
                    ExitCode::SUCCESS
                }
                Err(LoadError::Invalid { path, errors }) => {
                    for line in config_file::describe(&path, &errors) {
                        eprintln!("{line}");
                    }
                    let n = errors.0.len();
                    eprintln!("{n} error{}", if n == 1 { "" } else { "s" });
                    ExitCode::FAILURE
                }
                Err(e) => {
                    eprintln!("tidings: {e}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

/// Logging to stderr, filtered by `TIDINGS_LOG`, else `RUST_LOG`, else `info`.
fn init_logging() {
    let filter = EnvFilter::try_from_env("TIDINGS_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

fn ctl(request: Request) -> ExitCode {
    let Some(path) = ipc::socket_path() else {
        eprintln!("tidings: XDG_RUNTIME_DIR is not set");
        return ExitCode::FAILURE;
    };
    match client::send(&path, request) {
        Ok(Response::Ok(value)) => {
            if let Some(value) = value {
                println!("{value}");
            }
            ExitCode::SUCCESS
        }
        Ok(Response::Err(msg)) => {
            eprintln!("tidings: {msg}");
            ExitCode::FAILURE
        }
        Err(e @ ClientError::NotRunning(_)) => {
            eprintln!("tidings: {e}");
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("tidings: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Runs the daemon on a single-threaded tokio runtime until SIGINT/SIGTERM.
fn run_daemon(args: &DaemonArgs) -> anyhow::Result<()> {
    let socket_path = ipc::socket_path().context("XDG_RUNTIME_DIR is not set")?;
    let mut config = DaemonConfig::new(socket_path);
    config.settings = config_file::load_or_default(config_file::config_path().as_deref());
    config.colors = config_file::color_source(&config.settings);
    if let Some(source) = &config.colors {
        tracing::info!("colours: {source}");
    }
    if args.no_gestures {
        config.settings.gestures.enabled = false;
    }
    let animation = config.settings.animation;
    config.look = themes::resolve(&config.settings, themes::themes_dir().as_deref());
    let look = config.look.clone();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("cannot start the async runtime")?;
    runtime.block_on(async {
        let daemon = Daemon::start(
            config,
            Arc::new(SystemClock::default()),
            Box::new(move |events| spawn_ui(events, animation, &look)),
        )
        .await?;
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            r = tokio::signal::ctrl_c() => r?,
            _ = sigterm.recv() => {}
        }
        tracing::info!("shutting down");
        daemon.shutdown().await;
        Ok(())
    })
}

/// `tidings theme ...`: list, show, check, or print the themes folder.
fn theme(command: ThemeCommand) -> ExitCode {
    let dir = themes::themes_dir();
    match command {
        ThemeCommand::List => {
            let chosen = config_file::load_or_default(config_file::config_path().as_deref())
                .theme
                .name
                .unwrap_or_else(|| DEFAULT_THEME.to_owned());
            for e in themes::list(dir.as_deref()) {
                let mark = if e.name == chosen { '*' } else { ' ' };
                let origin = match &e.origin {
                    Origin::Builtin => "built-in".to_owned(),
                    Origin::File(p) => p.display().to_string(),
                };
                let about = e.description.unwrap_or_default();
                println!("{mark} {:<10} {about}  ({origin})", e.name);
            }
            ExitCode::SUCCESS
        }
        ThemeCommand::Show { name } => match themes::find(&name, dir.as_deref()) {
            Ok(Some(source)) => {
                print!("{}", source.text);
                ExitCode::SUCCESS
            }
            Ok(None) => {
                eprintln!("tidings: no theme called {name:?} (see `tidings theme list`)");
                ExitCode::FAILURE
            }
            Err(e) => {
                eprintln!("tidings: {e}");
                ExitCode::FAILURE
            }
        },
        ThemeCommand::Check { theme } => check_theme(theme, dir.as_deref()),
        ThemeCommand::Path => {
            if let Some(dir) = dir {
                println!("{}", dir.display());
                ExitCode::SUCCESS
            } else {
                eprintln!("tidings: neither XDG_CONFIG_HOME nor HOME is set");
                ExitCode::FAILURE
            }
        }
    }
}

/// Checks a theme given by name or path (default: the configured one).
fn check_theme(theme: Option<String>, dir: Option<&std::path::Path>) -> ExitCode {
    let theme = theme.unwrap_or_else(|| {
        config_file::load_or_default(config_file::config_path().as_deref())
            .theme
            .name
            .unwrap_or_else(|| DEFAULT_THEME.to_owned())
    });
    // Anything that cannot be a theme name is a path.
    let source = if tidings_core::config::theme::is_valid_name(&theme) {
        themes::find(&theme, dir)
    } else {
        let path = std::path::PathBuf::from(&theme);
        std::fs::read_to_string(&path)
            .map(|text| {
                Some(themes::Source {
                    name: theme.clone(),
                    origin: Origin::File(path.clone()),
                    text,
                })
            })
            .map_err(|source| LoadError::Io { path, source })
    };
    let source = match source {
        Ok(Some(s)) => s,
        Ok(None) => {
            eprintln!("tidings: no theme called {theme:?} (see `tidings theme list`)");
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("tidings: {e}");
            return ExitCode::FAILURE;
        }
    };
    match themes::parse(&source) {
        Ok(_) => {
            println!("{}: ok", source.display_path().display());
            ExitCode::SUCCESS
        }
        Err(LoadError::Invalid { path, errors }) => {
            for line in config_file::describe(&path, &errors) {
                eprintln!("{line}");
            }
            let n = errors.0.len();
            eprintln!("{n} error{}", if n == 1 { "" } else { "s" });
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("tidings: {e}");
            ExitCode::FAILURE
        }
    }
}
