//! Finding themes: the user's theme files and the built-in ones, and
//! resolving the config's choice into a [`Look`].
//!
//! A theme named `name` is `themes/<name>.toml` in the config directory if
//! that file exists, else the built-in theme of that name. So copying a
//! built-in into the themes folder and editing it overrides it.

use std::path::{Path, PathBuf};

use tidings_core::config::Config;
use tidings_core::config::theme::{self, DEFAULT_THEME, ThemeFile};
use tidings_core::look::Look;

use crate::config_file::{self, LoadError};

/// `themes` in [`config_file::config_dir`].
#[must_use]
pub fn themes_dir() -> Option<PathBuf> {
    Some(config_file::config_dir()?.join("themes"))
}

/// Where a theme comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// A file in the themes folder.
    File(PathBuf),
    /// Compiled into tidings.
    Builtin,
}

/// A theme's source text and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub name: String,
    pub origin: Origin,
    pub text: String,
}

/// Finds theme `name`: the user's file in `dir` first, then the built-ins.
pub fn find(name: &str, dir: Option<&Path>) -> Result<Option<Source>, LoadError> {
    if let Some(path) = dir.map(|d| d.join(format!("{name}.toml"))) {
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                return Ok(Some(Source {
                    name: name.to_owned(),
                    origin: Origin::File(path),
                    text,
                }));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(LoadError::Io { path, source }),
        }
    }
    Ok(theme::builtin(name).map(|text| Source {
        name: name.to_owned(),
        origin: Origin::Builtin,
        text: text.to_owned(),
    }))
}

/// Parses a theme's source; errors carry the file's path (or `builtin:name`).
pub fn parse(source: &Source) -> Result<ThemeFile, LoadError> {
    ThemeFile::from_toml(&source.text).map_err(|errors| LoadError::Invalid {
        path: source.display_path(),
        errors,
    })
}

impl Source {
    /// The file, or `builtin:<name>` for a built-in theme.
    #[must_use]
    pub fn display_path(&self) -> PathBuf {
        match &self.origin {
            Origin::File(p) => p.clone(),
            Origin::Builtin => PathBuf::from(format!("builtin:{}", self.name)),
        }
    }
}

/// A theme available to choose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub origin: Origin,
    /// The theme's own description, if it parses and has one.
    pub description: Option<String>,
}

/// Every theme by name, sorted: the built-ins and the `.toml` files in
/// `dir`. A file with a built-in's name replaces it.
#[must_use]
pub fn list(dir: Option<&Path>) -> Vec<Entry> {
    let describe = |text: &str| ThemeFile::from_toml(text).ok().and_then(|t| t.description);
    let mut entries: Vec<Entry> = theme::BUILTIN
        .iter()
        .map(|(name, text)| Entry {
            name: (*name).to_owned(),
            origin: Origin::Builtin,
            description: describe(text),
        })
        .collect();
    let files = dir
        .and_then(|d| std::fs::read_dir(d).ok())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"));
    for path in files {
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if !theme::is_valid_name(name) {
            continue;
        }
        let entry = Entry {
            name: name.to_owned(),
            description: std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| describe(&t)),
            origin: Origin::File(path.clone()),
        };
        match entries.iter_mut().find(|e| e.name == entry.name) {
            Some(existing) => *existing = entry,
            None => entries.push(entry),
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

/// The look for `config`: its theme (or the default) with the config's own
/// `[theme.*]` settings on top. A missing or broken theme is logged and the
/// default look used instead; the config's settings still apply.
#[must_use]
pub fn resolve(config: &Config, dir: Option<&Path>) -> Look {
    let name = config.theme.name.as_deref().unwrap_or(DEFAULT_THEME);
    let mut look = match find(name, dir).and_then(|s| s.map(|s| parse(&s)).transpose()) {
        Ok(Some(theme)) => {
            tracing::info!("theme: {name}");
            theme.look()
        }
        Ok(None) => {
            tracing::error!(
                "no theme called {name:?}; using {DEFAULT_THEME} (see `tidings theme list`)"
            );
            Look::default()
        }
        Err(LoadError::Invalid { path, errors }) => {
            for line in config_file::describe(&path, &errors) {
                tracing::error!("{line}");
            }
            tracing::error!("theme {name:?} has errors; using {DEFAULT_THEME}");
            Look::default()
        }
        Err(e) => {
            tracing::error!("{e}; using {DEFAULT_THEME}");
            Look::default()
        }
    };
    config.theme.look.apply(&mut look);
    look
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(src: &str) -> Config {
        Config::from_toml(src).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn user_files_override_builtins_and_add_themes() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let d = dir.path();
        std::fs::write(
            d.join("flat.toml"),
            "description = \"Mine\"\n[card]\nradius = 1\n",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(d.join("neon.toml"), "[panel]\nradius = 3\n")
            .unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(d.join("bad name.toml"), "").unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(d.join("notes.txt"), "").unwrap_or_else(|e| panic!("{e}"));

        let entries = list(Some(d));
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"neon") && names.contains(&"glass"));
        assert!(!names.iter().any(|n| n.contains(' ') || *n == "notes"));
        assert!(names.windows(2).all(|w| w[0] < w[1]), "sorted, no repeats");
        let flat = entries.iter().find(|e| e.name == "flat");
        assert_eq!(flat.and_then(|e| e.description.as_deref()), Some("Mine"));
        assert_eq!(
            flat.map(|e| e.origin.clone()),
            Some(Origin::File(d.join("flat.toml")))
        );

        let look = resolve(&config("[theme]\nname = \"flat\"\n"), Some(d));
        assert!((look.card.radius - 1.0).abs() < f32::EPSILON);
        let look = resolve(&config("[theme]\nname = \"neon\"\n"), Some(d));
        assert!((look.panel.radius - 3.0).abs() < f32::EPSILON);
    }

    #[test]
    fn resolve_layers_the_config_and_falls_back() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let d = dir.path();
        assert_eq!(resolve(&Config::default(), Some(d)), Look::default());
        assert_eq!(resolve(&Config::default(), None), Look::default());

        let look = resolve(
            &config("[theme]\nname = \"terminal\"\n[theme.panel]\nradius = 9\n"),
            Some(d),
        );
        assert!((look.panel.radius - 9.0).abs() < f32::EPSILON);
        assert!(!look.toast.blur, "the rest is the terminal theme");

        // Unknown or broken themes fall back, keeping the config's settings.
        let look = resolve(
            &config("[theme]\nname = \"nope\"\n[theme.card]\nradius = 5\n"),
            Some(d),
        );
        assert!((look.card.radius - 5.0).abs() < f32::EPSILON);
        assert_eq!(look.panel, Look::default().panel);
        std::fs::write(d.join("broken.toml"), "[panel]\nradius = 999\n")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            resolve(&config("[theme]\nname = \"broken\"\n"), Some(d)),
            Look::default()
        );
        let Ok(Some(src)) = find("broken", Some(d)) else {
            panic!("broken theme found");
        };
        assert!(matches!(parse(&src), Err(LoadError::Invalid { .. })));
    }

    #[test]
    fn an_unreadable_theme_file_falls_back_and_is_reported() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let d = dir.path();
        // A directory where the theme file should be.
        std::fs::create_dir(d.join("odd.toml")).unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            find("odd", Some(d)),
            Err(LoadError::Io { path, .. }) if path == d.join("odd.toml")
        ));
        let look = resolve(
            &config("[theme]\nname = \"odd\"\n[theme.card]\nradius = 5\n"),
            Some(d),
        );
        assert!((look.card.radius - 5.0).abs() < f32::EPSILON);
        assert_eq!(look.panel, Look::default().panel);
    }

    #[test]
    fn builtin_sources_are_named_by_their_theme() {
        let Ok(Some(src)) = find("nord", None) else {
            panic!("nord is built in");
        };
        assert_eq!(src.origin, Origin::Builtin);
        assert_eq!(src.display_path(), PathBuf::from("builtin:nord"));
        assert!(parse(&src).is_ok());
    }

    #[test]
    fn list_skips_files_whose_names_are_not_text() {
        use std::os::unix::ffi::OsStrExt as _;

        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let name = std::ffi::OsStr::from_bytes(b"\xff.toml");
        std::fs::write(dir.path().join(name), "").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(list(Some(dir.path())), list(None));
    }
}
