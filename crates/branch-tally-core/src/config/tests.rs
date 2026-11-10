use std::collections::HashMap;

use super::*;

#[derive(Default)]
struct Fake {
    vars: HashMap<String, OsString>,
    files: HashMap<PathBuf, io::Result<String>>,
}

impl Fake {
    fn with_file(text: &str) -> (Self, PathBuf) {
        let path = PathBuf::from("/cfg/branch-tally/config.toml");
        let mut f = Self::default();
        f.files.insert(path.clone(), Ok(text.to_owned()));
        (f, path)
    }

    fn with_vars(pairs: &[(&str, &str)]) -> Self {
        let mut f = Self::default();
        for (k, v) in pairs {
            f.vars.insert((*k).into(), (*v).into());
        }
        f
    }
}

impl ConfigEnv for Fake {
    fn var(&self, key: &str) -> Option<OsString> {
        self.vars.get(key).cloned()
    }
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        match self.files.get(path) {
            Some(Ok(s)) => Ok(s.clone()),
            Some(Err(e)) => Err(io::Error::new(e.kind(), "fake")),
            None => Err(io::ErrorKind::NotFound.into()),
        }
    }
}

fn load_text(text: &str, flags: &Flags) -> Result<Settings, ConfigError> {
    let (env, path) = Fake::with_file(text);
    load(&env, Some(&path), flags)
}

const FULL: &str =
    "mode = \"timeline\"\nwork-branch = \"^feat/\"\nformat = \"json\"\ncost = false\n";

#[test]
fn defaults_apply_with_no_file_and_no_flags() {
    let env = Fake::with_vars(&[("HOME", "/h"), ("APPDATA", "C:\\A")]);
    let s = load(&env, None, &Flags::default()).unwrap();
    assert!(!s.file_found);
    assert!(s.path.is_some());
    assert_eq!(
        s.mode,
        Resolved {
            value: Mode::Claim,
            source: Source::Default
        }
    );
    assert_eq!(
        s.format,
        Resolved {
            value: Format::Md,
            source: Source::Default
        }
    );
    assert_eq!(
        s.cost,
        Resolved {
            value: true,
            source: Source::Default
        }
    );
    assert_eq!(
        s.work_branch,
        Resolved {
            value: None,
            source: Source::Default
        }
    );
    assert!(!s.work_branch_active);
}

#[test]
fn config_beats_default_for_each_key() {
    let s = load_text(FULL, &Flags::default()).unwrap();
    assert!(s.file_found);
    assert_eq!(
        s.mode,
        Resolved {
            value: Mode::Timeline,
            source: Source::ConfigFile
        }
    );
    assert_eq!(
        s.work_branch,
        Resolved {
            value: Some("^feat/".into()),
            source: Source::ConfigFile
        }
    );
    assert_eq!(
        s.format,
        Resolved {
            value: Format::Json,
            source: Source::ConfigFile
        }
    );
    assert_eq!(
        s.cost,
        Resolved {
            value: false,
            source: Source::ConfigFile
        }
    );
    assert!(s.work_branch_active);
}

#[test]
fn flag_beats_config_for_each_key() {
    let flags = Flags {
        mode: Some(Mode::Strict),
        work_branch: Some("^fix/".into()),
        format: Some(Format::Md),
        cost: Some(true),
    };
    let s = load_text(FULL, &flags).unwrap();
    assert_eq!(
        s.mode,
        Resolved {
            value: Mode::Strict,
            source: Source::Flag
        }
    );
    assert_eq!(
        s.work_branch,
        Resolved {
            value: Some("^fix/".into()),
            source: Source::Flag
        }
    );
    assert_eq!(
        s.format,
        Resolved {
            value: Format::Md,
            source: Source::Flag
        }
    );
    assert_eq!(
        s.cost,
        Resolved {
            value: true,
            source: Source::Flag
        }
    );
}

#[test]
fn config_work_branch_is_inactive_unless_mode_is_timeline() {
    let s = load_text("work-branch = \"^feat/\"\n", &Flags::default()).unwrap();
    assert_eq!(s.work_branch.value.as_deref(), Some("^feat/"));
    assert!(!s.work_branch_active);
    let flags = Flags {
        mode: Some(Mode::Timeline),
        ..Flags::default()
    };
    let s = load_text("work-branch = \"^feat/\"\n", &flags).unwrap();
    assert!(s.work_branch_active);
}

#[test]
fn missing_default_file_is_normal_but_missing_explicit_file_is_an_error() {
    let env = Fake::with_vars(&[("HOME", "/h"), ("APPDATA", "C:\\A")]);
    assert!(load(&env, None, &Flags::default()).is_ok());
    let err = load(&env, Some(Path::new("/nope.toml")), &Flags::default()).unwrap_err();
    assert!(matches!(err, ConfigError::NotFound { .. }));
    assert!(err.to_string().contains("nope.toml"));
}

#[test]
fn unreadable_file_is_a_read_error_naming_the_file() {
    let mut env = Fake::default();
    let p = PathBuf::from("/cfg/c.toml");
    env.files
        .insert(p.clone(), Err(io::ErrorKind::PermissionDenied.into()));
    let err = load(&env, Some(&p), &Flags::default()).unwrap_err();
    assert!(matches!(err, ConfigError::Read { .. }));
    assert!(err.to_string().contains("c.toml"));
}

#[test]
fn malformed_toml_is_an_error_naming_the_file() {
    let err = load_text("mode = = nope", &Flags::default()).unwrap_err();
    assert!(matches!(err, ConfigError::Malformed { .. }));
    assert!(err.to_string().contains("config.toml"));
}

#[test]
fn unknown_key_is_an_error_naming_file_and_key() {
    let err = load_text("harness = \"claude\"\n", &Flags::default()).unwrap_err();
    assert!(matches!(err, ConfigError::UnknownKey { .. }));
    let msg = err.to_string();
    assert!(
        msg.contains("config.toml") && msg.contains("harness"),
        "{msg}"
    );
}

#[test]
fn bad_values_name_file_and_key() {
    for (text, key) in [
        ("mode = \"loose\"", "mode"),
        ("mode = 3", "mode"),
        ("format = \"xml\"", "format"),
        ("cost = \"yes\"", "cost"),
        ("work-branch = 7", "work-branch"),
    ] {
        let err = load_text(text, &Flags::default()).unwrap_err();
        assert!(matches!(err, ConfigError::BadValue { .. }), "{text}");
        let msg = err.to_string();
        assert!(msg.contains("config.toml") && msg.contains(key), "{msg}");
    }
}

#[test]
fn default_path_follows_platform_rules() {
    let tail = Path::new("branch-tally").join("config.toml");
    if cfg!(windows) {
        let got = default_config_path(&Fake::with_vars(&[("APPDATA", "C:\\A"), ("HOME", "C:\\H")]));
        assert_eq!(got, Some(Path::new("C:\\A").join(&tail)));
        assert_eq!(
            default_config_path(&Fake::with_vars(&[("HOME", "C:\\H")])),
            None
        );
        assert_eq!(
            default_config_path(&Fake::with_vars(&[("APPDATA", "")])),
            None
        );
    } else {
        let got = default_config_path(&Fake::with_vars(&[
            ("HOME", "/h"),
            ("XDG_CONFIG_HOME", "/x"),
        ]));
        assert_eq!(got, Some(Path::new("/x").join(&tail)));
        let got = default_config_path(&Fake::with_vars(&[("HOME", "/h"), ("XDG_CONFIG_HOME", "")]));
        assert_eq!(got, Some(Path::new("/h/.config").join(&tail)));
        let got = default_config_path(&Fake::with_vars(&[
            ("HOME", "/h"),
            ("XDG_CONFIG_HOME", "rel"),
        ]));
        assert_eq!(got, Some(Path::new("/h/.config").join(&tail)));
        assert_eq!(default_config_path(&Fake::with_vars(&[])), None);
    }
}

#[test]
fn default_config_text_parses_to_the_built_in_defaults() {
    let (env, path) = Fake::with_file(DEFAULT_CONFIG);
    let from_file = load(&env, Some(&path), &Flags::default()).expect("template loads");
    let built_in = load(&Fake::default(), None, &Flags::default()).expect("defaults load");
    assert!(from_file.file_found);
    assert_eq!(from_file.mode.value, built_in.mode.value);
    assert_eq!(from_file.format.value, built_in.format.value);
    assert_eq!(from_file.cost.value, built_in.cost.value);
    assert_eq!(from_file.work_branch.value, built_in.work_branch.value);
    // Every key with a default is written out, not left implicit.
    for r in [
        from_file.mode.source,
        from_file.format.source,
        from_file.cost.source,
    ] {
        assert_eq!(r, Source::ConfigFile);
    }
    // work-branch has no default: it is documented in a comment, never set.
    assert!(
        DEFAULT_CONFIG.contains("# work-branch = "),
        "{DEFAULT_CONFIG}"
    );
    assert!(!DEFAULT_CONFIG.contains('\r'), "LF only");
}
