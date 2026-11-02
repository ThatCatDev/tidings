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
use tidings_core::config::{AnimationConfig, TEMPLATE};
use tidings_core::ui::{UiEvent, UiHandle};
use tracing_subscriber::EnvFilter;

use clap::Parser as _;

use crate::cli::{Cli, Command, ConfigCommand, DaemonArgs};

/// The only place the UI is created: starts the Wayland UI thread.
///
/// `Daemon::start` calls this only after the bus name and IPC socket are
/// acquired, so a refused start never shows any surface.
fn spawn_ui(
    events: Sender<UiEvent>,
    animation: AnimationConfig,
) -> Result<Box<dyn UiHandle>, Box<dyn std::error::Error + Send + Sync>> {
    Ok(tidings_ui::spawn(events, animation)?)
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
    config.colors_path = config_file::colors_path(&config.settings);
    if args.no_gestures {
        config.settings.gestures.enabled = false;
    }
    let animation = config.settings.animation;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("cannot start the async runtime")?;
    runtime.block_on(async {
        let daemon = Daemon::start(
            config,
            Arc::new(SystemClock::default()),
            Box::new(move |events| spawn_ui(events, animation)),
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
