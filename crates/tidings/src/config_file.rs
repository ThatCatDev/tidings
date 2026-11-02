//! Finding and reading the user's files: `config.toml` and where
//! `colors.json` lives.
//!
//! Parsing and validation are in [`tidings_core::config`]; this module adds
//! the file system and environment.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use tidings_core::config::{Config, ConfigErrors};

/// A non-empty environment variable.
fn var(name: &str) -> Option<OsString> {
    std::env::var_os(name).filter(|v| !v.is_empty())
}

/// `$XDG_CONFIG_HOME/tidings`, else `~/.config/tidings`.
#[must_use]
pub fn config_dir() -> Option<PathBuf> {
    let base = var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("tidings"))
}

/// The config file: `$TIDINGS_CONFIG`, else `config.toml` in [`config_dir`].
#[must_use]
pub fn config_path() -> Option<PathBuf> {
    var("TIDINGS_CONFIG")
        .map(PathBuf::from)
        .or_else(|| Some(config_dir()?.join("config.toml")))
}

/// The colours file: `$TIDINGS_COLORS`, else the config's `theme.colors`,
/// else `colors.json` in [`config_dir`].
#[must_use]
pub fn colors_path(config: &Config) -> Option<PathBuf> {
    if let Some(p) = var("TIDINGS_COLORS") {
        return Some(PathBuf::from(p));
    }
    if let Some(p) = &config.theme.colors {
        return Some(expand_home(p, var("HOME").map(PathBuf::from).as_deref()));
    }
    Some(config_dir()?.join("colors.json"))
}

/// `~/rest` with `home` substituted; other paths unchanged.
#[must_use]
pub fn expand_home(path: &Path, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix("~"), home) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => path.to_path_buf(),
    }
}

/// Why a config file could not be used.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// There is no file (not an error for the daemon: defaults apply).
    #[error("{0}: no such file")]
    Missing(PathBuf),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The file has problems; each is listed with its position.
    #[error("{path}: invalid config")]
    Invalid { path: PathBuf, errors: ConfigErrors },
}

/// Reads and validates the config file at `path`.
pub fn load(path: &Path) -> Result<Config, LoadError> {
    let text = std::fs::read_to_string(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            LoadError::Missing(path.to_path_buf())
        } else {
            LoadError::Io {
                path: path.to_path_buf(),
                source,
            }
        }
    })?;
    Config::from_toml(&text).map_err(|errors| LoadError::Invalid {
        path: path.to_path_buf(),
        errors,
    })
}

/// Each problem as `path:line:column: key: message`, the format editors and
/// terminals recognise.
#[must_use]
pub fn describe(path: &Path, errors: &ConfigErrors) -> Vec<String> {
    errors
        .0
        .iter()
        .map(|e| {
            let key = e.key.as_ref().map_or(String::new(), |k| format!("{k}: "));
            format!(
                "{}:{}:{}: {key}{}",
                path.display(),
                e.line,
                e.column,
                e.message
            )
        })
        .collect()
}

/// The daemon's settings: the config file if it is valid, otherwise the
/// defaults. Problems are logged; a bad file never stops the daemon.
#[must_use]
pub fn load_or_default(path: Option<&Path>) -> Config {
    let Some(path) = path else {
        return Config::default();
    };
    match load(path) {
        Ok(config) => {
            tracing::info!("config: {}", path.display());
            config
        }
        Err(LoadError::Missing(_)) => Config::default(),
        Err(LoadError::Invalid { path, errors }) => {
            for line in describe(&path, &errors) {
                tracing::error!("{line}");
            }
            tracing::error!(
                "config has errors; using the defaults (check with `tidings config check`)"
            );
            Config::default()
        }
        Err(e) => {
            tracing::error!("{e}; using the default config");
            Config::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_is_expanded_only_at_the_start() {
        let home = Path::new("/home/me");
        assert_eq!(
            expand_home(Path::new("~/x/colors.json"), Some(home)),
            PathBuf::from("/home/me/x/colors.json")
        );
        assert_eq!(
            expand_home(Path::new("/etc/~/c"), Some(home)),
            PathBuf::from("/etc/~/c")
        );
        assert_eq!(expand_home(Path::new("~/c"), None), PathBuf::from("~/c"));
    }

    #[test]
    fn load_reports_missing_invalid_and_valid_files() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("config.toml");
        assert!(matches!(load(&path), Err(LoadError::Missing(_))));
        assert_eq!(load_or_default(Some(&path)), Config::default());

        std::fs::write(&path, "[toasts]\nmax_visible = 99\n").unwrap_or_else(|e| panic!("{e}"));
        let Err(LoadError::Invalid { errors, .. }) = load(&path) else {
            panic!("expected invalid");
        };
        let lines = describe(&path, &errors);
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].ends_with(":2:15: toasts.max_visible: 99 is out of range (1 to 10)"),
            "{}",
            lines[0]
        );
        assert_eq!(load_or_default(Some(&path)), Config::default());

        std::fs::write(&path, "[toasts]\nmax_visible = 2\n").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(load(&path).map(|c| c.toasts.max_visible).ok(), Some(2));
    }
}
