pub const USAGE: &str = "Usage: montty [--profile NAME] [--session NAME] [-e COMMAND [ARGS...]]\n\n  --profile NAME  the window's default profile (profiles/NAME.toml): the first pane\n                  and new tabs use it\n  --session NAME  open the saved session sessions/NAME.json (not with -e)\n  -e, --command   run COMMAND (everything after it) instead of the default shell\n  -h, --help      print this help";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    pub command: Option<Vec<String>>,
    /// `--profile`: the window's default profile stem.
    pub profile: Option<String>,
    /// `--session`: the saved session to open at startup.
    pub session: Option<String>,
    /// Set when -h/--help was given; the caller prints usage and exits 0.
    pub help: bool,
}

pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Args, String> {
    let mut it = args.into_iter();
    let mut out = Args::default();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => {
                out.help = true;
                return Ok(out);
            }
            "--profile" => {
                // `--profile -e cmd` forgot the name; `-e` is not a profile.
                let stem = it.next().filter(|s| !s.starts_with('-'));
                out.profile = Some(stem.ok_or_else(|| format!("{a} requires a profile name"))?);
            }
            "--session" => {
                let name = it.next().filter(|s| !s.starts_with('-'));
                out.session = Some(name.ok_or_else(|| format!("{a} requires a session name"))?);
            }
            // Must come last: everything after it is the command.
            "-e" | "--command" => {
                let cmd: Vec<String> = it.collect();
                if cmd.is_empty() {
                    return Err(format!("{a} requires a command"));
                }
                if out.session.is_some() {
                    return Err("--session and -e can't be combined".into());
                }
                out.command = Some(cmd);
                return Ok(out);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_args_means_default_shell() {
        assert_eq!(parse_args(vec![]).unwrap().command, None);
    }
    #[test]
    fn dash_e_takes_rest_as_command() {
        let a = parse_args(["-e", "sh", "-c", "echo hi"].map(String::from)).unwrap();
        assert_eq!(a.command.unwrap(), vec!["sh", "-c", "echo hi"]);
    }
    #[test]
    fn dash_e_without_command_is_error() {
        assert!(parse_args(["-e".to_string()]).is_err());
    }
    #[test]
    fn unknown_flag_is_error() {
        assert!(parse_args(["--bogus".to_string()]).is_err());
    }
    #[test]
    fn cli_profile_flag() {
        let a = parse_args(["--profile", "work"].map(String::from)).unwrap();
        assert_eq!(a.profile.as_deref(), Some("work"));
        assert_eq!(a.command, None);
        let a = parse_args(["--profile", "work", "-e", "htop"].map(String::from)).unwrap();
        assert_eq!(a.profile.as_deref(), Some("work"));
        assert_eq!(a.command.unwrap(), vec!["htop"]);
        assert!(parse_args(["--profile".to_string()]).is_err());
        // -e takes everything after it, --profile included.
        let a = parse_args(["-e", "echo", "--profile", "x"].map(String::from)).unwrap();
        assert_eq!(a.profile, None);
        assert_eq!(a.command.unwrap(), vec!["echo", "--profile", "x"]);
        assert!(USAGE.contains("--profile"));
        // A flag is not a profile name.
        let e = parse_args(["--profile", "-e", "htop"].map(String::from)).unwrap_err();
        assert_eq!(e, "--profile requires a profile name");
        assert!(parse_args(["--profile", "--help"].map(String::from)).is_err());
    }

    #[test]
    fn cli_session_flag() {
        let a = parse_args(["--session", "work"].map(String::from)).unwrap();
        assert_eq!(a.session.as_deref(), Some("work"));
        assert_eq!(a.command, None);
        assert_eq!(parse_args(vec![]).unwrap().session, None);
        let e = parse_args(["--session"].map(String::from)).unwrap_err();
        assert_eq!(e, "--session requires a session name");
        assert!(parse_args(["--session", "-e", "x"].map(String::from)).is_err());
        assert!(USAGE.contains("--session"));
    }

    #[test]
    fn cli_session_with_e_is_error() {
        let e = parse_args(["--session", "work", "-e", "htop"].map(String::from)).unwrap_err();
        assert_eq!(e, "--session and -e can't be combined");
    }

    #[test]
    fn help_flag_sets_help() {
        assert!(parse_args(["-h".to_string()]).unwrap().help);
    }
}
