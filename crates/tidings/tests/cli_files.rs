//! Tests of the `tidings config ...` and `tidings theme ...` commands: what
//! they print, their exit codes, and where they look for files.
//!
//! Every run gets a temporary `HOME` (see [`common::tidings_cmd`]), so the
//! user's own config and themes are never read.

#![allow(clippy::unwrap_used)] // test helpers

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// A temporary home directory, with helpers for its tidings files.
struct Home(TempDir);

impl Home {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }

    fn path(&self) -> &Path {
        self.0.path()
    }

    /// `~/.config/tidings`.
    fn config_dir(&self) -> PathBuf {
        self.path().join(".config/tidings")
    }

    fn themes_dir(&self) -> PathBuf {
        self.config_dir().join("themes")
    }

    /// Writes `~/.config/tidings/<rel>`, creating its folders.
    fn write(&self, rel: &str, text: &str) -> PathBuf {
        let path = self.config_dir().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        path
    }

    fn cmd(&self) -> Command {
        common::tidings_cmd(self.path())
    }

    fn run(&self, args: &[&str]) -> Run {
        Run::of(self.cmd().args(args))
    }
}

/// A finished run's exit code and output.
#[derive(Debug)]
struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Run {
    fn of(cmd: &mut Command) -> Self {
        let out = cmd.output().unwrap();
        Self {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }
}

/// Runs with neither `HOME` nor `XDG_CONFIG_HOME`, so no config folder exists.
fn run_homeless(args: &[&str]) -> Run {
    let home = Home::new();
    Run::of(home.cmd().env_remove("HOME").args(args))
}

#[test]
fn config_default_prints_the_template() {
    let run = Home::new().run(&["config", "default"]);
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, tidings_core::config::TEMPLATE);
}

#[test]
fn config_path_prefers_tidings_config_then_xdg_then_home() {
    let home = Home::new();
    let run = home.run(&["config", "path"]);
    assert_eq!(run.code, Some(0));
    assert_eq!(
        run.stdout.trim_end(),
        home.config_dir().join("config.toml").display().to_string()
    );

    let xdg = home.path().join("xdg");
    let run = Run::of(
        home.cmd()
            .env("XDG_CONFIG_HOME", &xdg)
            .args(["config", "path"]),
    );
    assert_eq!(
        run.stdout.trim_end(),
        xdg.join("tidings/config.toml").display().to_string()
    );

    let run = Run::of(
        home.cmd()
            .env("TIDINGS_CONFIG", "/somewhere/else.toml")
            .args(["config", "path"]),
    );
    assert_eq!(run.stdout, "/somewhere/else.toml\n");
}

#[test]
fn config_commands_fail_without_a_home() {
    for args in [["config", "path"], ["config", "check"]] {
        let run = run_homeless(&args);
        assert_eq!(run.code, Some(1), "{args:?}");
        assert!(
            run.stderr
                .contains("neither XDG_CONFIG_HOME nor HOME is set"),
            "{args:?}: {}",
            run.stderr
        );
    }
}

#[test]
fn config_check_accepts_a_missing_file() {
    let home = Home::new();
    let run = home.run(&["config", "check"]);
    assert_eq!(run.code, Some(0), "the defaults apply");
    let expected = home.config_dir().join("config.toml");
    assert_eq!(
        run.stdout,
        format!("{}: no such file; the defaults apply\n", expected.display())
    );
}

#[test]
fn config_check_accepts_a_valid_file() {
    let home = Home::new();
    let path = home.write("config.toml", "[toasts]\nmax_visible = 2\n");
    let run = home.run(&["config", "check"]);
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, format!("{}: ok\n", path.display()));
}

#[test]
fn config_check_lists_each_error_with_its_position() {
    let home = Home::new();
    let one = home.write("one.toml", "[toasts]\nmax_visible = 99\n");
    let run = home.run(&["config", "check", one.to_str().unwrap()]);
    assert_eq!(run.code, Some(1));
    assert_eq!(
        run.stderr,
        format!(
            "{}:2:15: toasts.max_visible: 99 is out of range (1 to 10)\n1 error\n",
            one.display()
        )
    );

    let two = home.write(
        "two.toml",
        "[toasts]\nmax_visible = 99\n[theme.panel]\nradius = 999\n",
    );
    let run = home.run(&["config", "check", two.to_str().unwrap()]);
    assert_eq!(run.code, Some(1));
    assert!(run.stderr.ends_with("\n2 errors\n"), "{}", run.stderr);
}

#[test]
fn config_check_reports_an_unreadable_file() {
    let home = Home::new();
    // A directory where the file should be cannot be read as one.
    std::fs::create_dir_all(home.config_dir().join("config.toml")).unwrap();
    let run = home.run(&["config", "check"]);
    assert_eq!(run.code, Some(1));
    assert!(run.stderr.starts_with("tidings: "), "{}", run.stderr);
    assert!(run.stderr.contains("config.toml"), "{}", run.stderr);
}

#[test]
fn theme_path_is_the_themes_folder() {
    let home = Home::new();
    let run = home.run(&["theme", "path"]);
    assert_eq!(run.code, Some(0));
    assert_eq!(
        run.stdout.trim_end(),
        home.themes_dir().display().to_string()
    );

    let run = run_homeless(&["theme", "path"]);
    assert_eq!(run.code, Some(1));
    assert!(
        run.stderr
            .contains("neither XDG_CONFIG_HOME nor HOME is set")
    );
}

/// The `theme list` line for theme `name` (after the two-column mark).
fn listed<'a>(stdout: &'a str, name: &str) -> &'a str {
    stdout
        .lines()
        .find(|l| l.get(2..).and_then(|rest| rest.split_whitespace().next()) == Some(name))
        .unwrap_or_else(|| panic!("{name} not listed in:\n{stdout}"))
}

#[test]
fn theme_list_marks_the_default_theme() {
    let run = Home::new().run(&["theme", "list"]);
    assert_eq!(run.code, Some(0));
    let glass = listed(&run.stdout, "glass");
    assert!(glass.starts_with("* glass"), "{glass}");
    assert!(glass.ends_with("(built-in)"), "{glass}");
    assert_eq!(run.stdout.lines().filter(|l| l.starts_with('*')).count(), 1);
}

#[test]
fn theme_list_shows_user_files_and_the_configured_theme() {
    let home = Home::new();
    home.write("config.toml", "[theme]\nname = \"neon\"\n");
    let neon = home.write("themes/neon.toml", "description = \"Bright\"\n");
    let run = home.run(&["theme", "list"]);
    assert_eq!(run.code, Some(0));
    let line = listed(&run.stdout, "neon");
    assert_eq!(line, format!("* neon       Bright  ({})", neon.display()));
    let glass = listed(&run.stdout, "glass");
    assert!(glass.starts_with("  glass"), "no longer chosen: {glass}");
}

#[test]
fn theme_show_prints_a_builtin_or_user_theme() {
    let home = Home::new();
    let run = home.run(&["theme", "show", "glass"]);
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, include_str!("../../../themes/glass.toml"));

    home.write("themes/glass.toml", "[panel]\nradius = 3\n");
    let run = home.run(&["theme", "show", "glass"]);
    assert_eq!(run.stdout, "[panel]\nradius = 3\n", "the user's file wins");
}

#[test]
fn theme_show_reports_unknown_and_unreadable_themes() {
    let home = Home::new();
    let run = home.run(&["theme", "show", "nope"]);
    assert_eq!(run.code, Some(1));
    assert!(
        run.stderr.contains("no theme called \"nope\""),
        "{}",
        run.stderr
    );

    std::fs::create_dir_all(home.themes_dir().join("odd.toml")).unwrap();
    let run = home.run(&["theme", "show", "odd"]);
    assert_eq!(run.code, Some(1));
    assert!(run.stderr.contains("odd.toml"), "{}", run.stderr);
}

#[test]
fn theme_check_by_name_defaults_to_the_configured_theme() {
    let home = Home::new();
    let run = home.run(&["theme", "check"]);
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, "builtin:glass: ok\n");

    home.write("config.toml", "[theme]\nname = \"nord\"\n");
    let run = home.run(&["theme", "check"]);
    assert_eq!(run.stdout, "builtin:nord: ok\n");

    let mine = home.write("themes/mine.toml", "[panel]\nradius = 3\n");
    let run = home.run(&["theme", "check", "mine"]);
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, format!("{}: ok\n", mine.display()));
}

#[test]
fn theme_check_accepts_a_path() {
    let home = Home::new();
    let path = home.path().join("elsewhere.toml");
    std::fs::write(&path, "[card]\nradius = 4\n").unwrap();
    let run = home.run(&["theme", "check", path.to_str().unwrap()]);
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, format!("{}: ok\n", path.display()));
}

#[test]
fn theme_check_lists_each_error() {
    let home = Home::new();
    let one = home.write("themes/one.toml", "[panel]\nradius = 999\n");
    let run = home.run(&["theme", "check", "one"]);
    assert_eq!(run.code, Some(1));
    assert!(
        run.stderr.starts_with(&format!("{}:2:", one.display())),
        "{}",
        run.stderr
    );
    assert!(run.stderr.ends_with("\n1 error\n"), "{}", run.stderr);

    home.write(
        "themes/two.toml",
        "[panel]\nradius = 999\nborder_width = 999\n",
    );
    let run = home.run(&["theme", "check", "two"]);
    assert_eq!(run.code, Some(1));
    assert!(run.stderr.ends_with("\n2 errors\n"), "{}", run.stderr);
}

#[test]
fn theme_check_reports_missing_and_unreadable_themes() {
    let home = Home::new();
    let run = home.run(&["theme", "check", "nope"]);
    assert_eq!(run.code, Some(1));
    assert!(
        run.stderr.contains("no theme called \"nope\""),
        "{}",
        run.stderr
    );

    let missing = home.path().join("missing.toml");
    let run = home.run(&["theme", "check", missing.to_str().unwrap()]);
    assert_eq!(run.code, Some(1));
    assert!(
        run.stderr
            .starts_with(&format!("tidings: {}: ", missing.display())),
        "{}",
        run.stderr
    );

    std::fs::create_dir_all(home.themes_dir().join("odd.toml")).unwrap();
    let run = home.run(&["theme", "check", "odd"]);
    assert_eq!(run.code, Some(1));
    assert!(run.stderr.contains("odd.toml"), "{}", run.stderr);
}
