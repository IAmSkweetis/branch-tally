//! The user config file: the real environment for core's loader, and `config show`.

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::Path;

use branch_tally_core::config::{self, ConfigEnv, Flags, Resolved, Settings, Source};

use crate::cli::{ConfigInitArgs, ConfigShowArgs};

/// The real machine: `std::env` and `std::fs`. Core never touches either itself.
pub(crate) struct SystemEnv;

impl ConfigEnv for SystemEnv {
    fn var(&self, key: &str) -> Option<OsString> {
        std::env::var_os(key)
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }
}

/// `config show`: the path, whether it was found, and each effective value with its source.
/// Plain text, LF endings, one setting per line.
pub(crate) fn show(args: &ConfigShowArgs) -> Result<String, String> {
    let settings = config::load(&SystemEnv, args.config.as_deref(), &Flags::default())
        .map_err(|e| e.to_string())?;
    Ok(render(&settings))
}

/// `config init`: write [`config::DEFAULT_CONFIG`] to `--path` or the user config path.
/// Creates missing parent directories and refuses to replace an existing file.
pub(crate) fn init(args: &ConfigInitArgs) -> Result<String, String> {
    let path = match &args.path {
        Some(p) => p.clone(),
        None => config::default_config_path(&SystemEnv).ok_or(
            "Could not determine the config directory (set APPDATA, XDG_CONFIG_HOME or HOME,              or pass --path)",
        )?,
    };
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    // create_new: an existing file is an error, never a silent overwrite.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| match e.kind() {
            io::ErrorKind::AlreadyExists => {
                format!(
                    "config file {} already exists; not overwritten",
                    path.display()
                )
            }
            _ => format!("cannot create config file {}: {e}", path.display()),
        })?;
    file.write_all(config::DEFAULT_CONFIG.as_bytes())
        .map_err(|e| format!("cannot write config file {}: {e}", path.display()))?;
    Ok(format!(
        "wrote {}
",
        path.display()
    ))
}

fn source_label(source: Source) -> &'static str {
    match source {
        Source::Flag => "flag",
        Source::ConfigFile => "config file",
        Source::Default => "default",
    }
}

fn line(name: &str, value: &str, resolved_source: Source) -> String {
    format!("{name} = {value} ({})", source_label(resolved_source))
}

fn render(settings: &Settings) -> String {
    let path = settings.path.as_ref().map_or_else(
        || "(none: no config directory could be determined)".to_owned(),
        |p| p.display().to_string(),
    );
    let Resolved { value, source } = &settings.work_branch;
    let mut work_branch = line(
        "work-branch",
        value.as_deref().unwrap_or("(not set)"),
        *source,
    );
    if value.is_some() && !settings.work_branch_active {
        work_branch.push_str(" [inactive: mode is not timeline]");
    }
    let lines = [
        format!("config file: {path}"),
        format!("found: {}", if settings.file_found { "yes" } else { "no" }),
        line("mode", settings.mode.value.as_str(), settings.mode.source),
        work_branch,
        line(
            "format",
            settings.format.value.as_str(),
            settings.format.source,
        ),
        line(
            "cost",
            &settings.cost.value.to_string(),
            settings.cost.source,
        ),
    ];
    format!("{}\n", lines.join("\n"))
}
