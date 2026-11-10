//! The user config file: default values for `report`'s flags.
//!
//! Precedence is CLI flag, then config file, then built-in default. Every resolved value carries
//! its [`Source`] so `config show` can say where it came from.
//!
//! Core never looks up a home directory or an environment variable itself. The lookup and the file
//! read go through [`ConfigEnv`]; the CLI supplies the real implementation (`std::env` and
//! `std::fs`) and tests supply a fake.
//!
//! The file is TOML with four optional, kebab-case keys: `mode` (`"claim"`, `"strict"`,
//! `"timeline"`), `work-branch` (a regex, kept as a string; the CLI compiles it), `format`
//! (`"md"`, `"json"`) and `cost` (`true` or `false`). A malformed file, an unknown key or a bad
//! value is a [`ConfigError`] naming the file and, where there is one, the key. Nothing is ignored
//! silently. Config content is never echoed except the offending key name.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use crate::model::Mode;

/// Output format of `report`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Format {
    /// Markdown (the default).
    #[default]
    Md,
    /// JSON.
    Json,
}

impl Format {
    /// The config/CLI spelling: `"md"` or `"json"`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Md => "md",
            Self::Json => "json",
        }
    }
}

/// Where a resolved value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A command-line flag.
    Flag,
    /// The user config file.
    ConfigFile,
    /// The built-in default.
    Default,
}

/// A value paired with the layer that decided it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved<T> {
    /// The effective value.
    pub value: T,
    /// Which layer supplied it.
    pub source: Source,
}

/// What the CLI needs from the machine: environment variables and file reads.
pub trait ConfigEnv {
    /// An environment variable, or `None` when unset.
    fn var(&self, key: &str) -> Option<OsString>;
    /// The whole text of a file. A missing file must surface as [`io::ErrorKind::NotFound`].
    ///
    /// # Errors
    /// Any I/O error from reading the file.
    fn read_to_string(&self, path: &Path) -> io::Result<String>;
}

/// Why the config could not be loaded. Every variant names the file; key problems name the key.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file exists but could not be read.
    #[error("cannot read config file {}: {message}", path.display())]
    Read {
        /// The file.
        path: PathBuf,
        /// The I/O error text.
        message: String,
    },
    /// An explicit `--config` path does not exist.
    #[error("config file {} does not exist", path.display())]
    NotFound {
        /// The file.
        path: PathBuf,
    },
    /// The text is not valid TOML.
    #[error("config file {} is not valid TOML: {message}", path.display())]
    Malformed {
        /// The file.
        path: PathBuf,
        /// The parser's message.
        message: String,
    },
    /// A key core does not know.
    #[error("config file {}: unknown key `{key}` (expected mode, work-branch, format, cost)", path.display())]
    UnknownKey {
        /// The file.
        path: PathBuf,
        /// The offending key.
        key: String,
    },
    /// A known key with the wrong type or an unknown choice.
    #[error("config file {}: key `{key}` {expected}", path.display())]
    BadValue {
        /// The file.
        path: PathBuf,
        /// The key.
        key: &'static str,
        /// What was expected.
        expected: &'static str,
    },
}

/// The parsed file: each key is `None` when absent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigFile {
    /// `mode`.
    pub mode: Option<Mode>,
    /// `work-branch`, an uncompiled regex.
    pub work_branch: Option<String>,
    /// `format`.
    pub format: Option<Format>,
    /// `cost`.
    pub cost: Option<bool>,
}

impl ConfigFile {
    /// Parse config text. `path` is only used to name the file in errors.
    ///
    /// # Errors
    /// [`ConfigError::Malformed`], [`ConfigError::UnknownKey`] or [`ConfigError::BadValue`].
    pub fn parse(text: &str, path: &Path) -> Result<Self, ConfigError> {
        let table: toml::Table =
            text.parse()
                .map_err(|e: toml::de::Error| ConfigError::Malformed {
                    path: path.to_owned(),
                    message: e.message().to_owned(),
                })?;
        let bad = |key: &'static str, expected: &'static str| ConfigError::BadValue {
            path: path.to_owned(),
            key,
            expected,
        };
        let mut out = Self::default();
        for (key, value) in &table {
            match key.as_str() {
                "mode" => {
                    out.mode = Some(match value.as_str() {
                        Some("claim") => Mode::Claim,
                        Some("strict") => Mode::Strict,
                        Some("timeline") => Mode::Timeline,
                        _ => {
                            return Err(bad(
                                "mode",
                                "must be \"claim\", \"strict\" or \"timeline\"",
                            ));
                        }
                    });
                }
                "work-branch" => {
                    let s = value
                        .as_str()
                        .ok_or_else(|| bad("work-branch", "must be a string (a regex)"))?;
                    out.work_branch = Some(s.to_owned());
                }
                "format" => {
                    out.format = Some(match value.as_str() {
                        Some("md") => Format::Md,
                        Some("json") => Format::Json,
                        _ => return Err(bad("format", "must be \"md\" or \"json\"")),
                    });
                }
                "cost" => {
                    out.cost = Some(
                        value
                            .as_bool()
                            .ok_or_else(|| bad("cost", "must be true or false"))?,
                    );
                }
                _ => {
                    return Err(ConfigError::UnknownKey {
                        path: path.to_owned(),
                        key: key.clone(),
                    });
                }
            }
        }
        Ok(out)
    }
}

/// Flag values from the command line; `None` means the flag was not given.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Flags {
    /// `--mode` (or `--strict` / `--timeline`).
    pub mode: Option<Mode>,
    /// `--work-branch`.
    pub work_branch: Option<String>,
    /// `--format` (or `--md` / `--json`).
    pub format: Option<Format>,
    /// `--cost` / `--no-cost`.
    pub cost: Option<bool>,
}

/// Effective settings plus how they were found. This is everything `config show` prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The file consulted, or `None` when no config directory could be determined.
    pub path: Option<PathBuf>,
    /// Whether that file existed and was read.
    pub file_found: bool,
    /// Effective attribution mode. Default: [`Mode::Claim`].
    pub mode: Resolved<Mode>,
    /// Effective work-branch regex source; `None` (source `Default`) when unset.
    pub work_branch: Resolved<Option<String>>,
    /// Whether the work-branch applies: true only when one is set and the effective mode is
    /// [`Mode::Timeline`]. A configured but inactive work-branch is ignored, not an error.
    pub work_branch_active: bool,
    /// Effective output format. Default: [`Format::Md`].
    pub format: Resolved<Format>,
    /// Whether to cost the report. Default: `true`.
    pub cost: Resolved<bool>,
}

/// The default config path: Windows `%APPDATA%\branch-tally\config.toml`; elsewhere
/// `$XDG_CONFIG_HOME/branch-tally/config.toml` when set, absolute and non-empty, else
/// `$HOME/.config/branch-tally/config.toml`. `None` when the needed variables are unset.
#[must_use]
pub fn default_config_path(env: &dyn ConfigEnv) -> Option<PathBuf> {
    let non_empty = |key: &str| env.var(key).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = if cfg!(windows) {
        non_empty("APPDATA")?
    } else {
        match non_empty("XDG_CONFIG_HOME").filter(|p| p.is_absolute()) {
            Some(dir) => dir,
            None => non_empty("HOME")?.join(".config"),
        }
    };
    Some(base.join("branch-tally").join("config.toml"))
}

fn pick<T>(flag: Option<T>, file: Option<T>, default: T) -> Resolved<T> {
    match (flag, file) {
        (Some(value), _) => Resolved {
            value,
            source: Source::Flag,
        },
        (None, Some(value)) => Resolved {
            value,
            source: Source::ConfigFile,
        },
        (None, None) => Resolved {
            value: default,
            source: Source::Default,
        },
    }
}

/// Resolve every setting: flag, then config file, then default.
///
/// `explicit` is `--config <path>`; when given it replaces the default location and a missing file
/// is [`ConfigError::NotFound`]. A missing file at the default location is normal.
///
/// # Errors
/// Any [`ConfigError`].
pub fn load(
    env: &dyn ConfigEnv,
    explicit: Option<&Path>,
    flags: &Flags,
) -> Result<Settings, ConfigError> {
    let path = explicit
        .map(Path::to_owned)
        .or_else(|| default_config_path(env));
    let mut file_found = false;
    let mut file = ConfigFile::default();
    if let Some(p) = &path {
        match env.read_to_string(p) {
            Ok(text) => {
                file = ConfigFile::parse(&text, p)?;
                file_found = true;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if explicit.is_some() {
                    return Err(ConfigError::NotFound { path: p.clone() });
                }
            }
            Err(e) => {
                return Err(ConfigError::Read {
                    path: p.clone(),
                    message: e.to_string(),
                });
            }
        }
    }
    let mode = pick(flags.mode, file.mode, Mode::Claim);
    let work_branch = match (flags.work_branch.clone(), file.work_branch) {
        (Some(v), _) => Resolved {
            value: Some(v),
            source: Source::Flag,
        },
        (None, Some(v)) => Resolved {
            value: Some(v),
            source: Source::ConfigFile,
        },
        (None, None) => Resolved {
            value: None,
            source: Source::Default,
        },
    };
    let work_branch_active = work_branch.value.is_some() && mode.value == Mode::Timeline;
    Ok(Settings {
        path,
        file_found,
        mode,
        work_branch,
        work_branch_active,
        format: pick(flags.format, file.format, Format::Md),
        cost: pick(flags.cost, file.cost, true),
    })
}

#[cfg(test)]
mod tests;
