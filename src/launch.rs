//! What a pane takes from montty's own environment, which is thin when Finder or the Dock
//! started it: no `$SHELL`, no locale, and `/` as the working directory.

use std::path::{Path, PathBuf};

/// `$SHELL` when set and non-empty, else the account's shell.
pub fn pick_shell(env: Option<String>, account: impl FnOnce() -> Option<String>) -> Option<String> {
    env.filter(|s| !s.is_empty()).or_else(account)
}

/// The shell for panes that don't name one (see `pick_shell`).
pub fn env_shell() -> Option<String> {
    pick_shell(std::env::var("SHELL").ok(), account_shell)
}

/// The login shell in the user's account record (`getpwuid_r`).
pub fn account_shell() -> Option<String> {
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut pw = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut out: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: `pw` and `buf` are valid for writes of their sizes; on success `out` points to
    // `pw`, whose strings point into `buf`, and both outlive the reads below.
    let rc = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            pw.as_mut_ptr(),
            buf.as_mut_ptr(),
            buf.len(),
            &mut out,
        )
    };
    if rc != 0 || out.is_null() {
        return None;
    }
    // SAFETY: as above; `pw_shell` is NUL-terminated (or null for no shell).
    let shell = unsafe {
        let p = (*out).pw_shell;
        if p.is_null() {
            return None;
        }
        std::ffi::CStr::from_ptr(p)
    };
    shell
        .to_str()
        .ok()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// `LC_CTYPE=UTF-8` when none of `LC_ALL`, `LC_CTYPE` and `LANG` is set, as Terminal.app
/// does; without it, bash and vim treat input as ASCII. Applied on macOS only.
pub fn locale_override(
    get: impl Fn(&str) -> Option<String>,
) -> Option<(&'static str, &'static str)> {
    let set = |k: &str| get(k).is_some_and(|v| !v.is_empty());
    (!["LC_ALL", "LC_CTYPE", "LANG"].into_iter().any(set)).then_some(("LC_CTYPE", "UTF-8"))
}

/// `home` when montty runs in `/` (a Finder or Dock launch): a pane with no directory of its
/// own starts there rather than in `/`.
pub fn start_dir(process_cwd: Option<&Path>, home: Option<PathBuf>) -> Option<PathBuf> {
    if process_cwd == Some(Path::new("/")) {
        home
    } else {
        None
    }
}

/// `start_dir` for this process, on macOS; None elsewhere (a desktop launcher starts in $HOME).
pub fn default_start_dir() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    start_dir(
        std::env::current_dir().ok().as_deref(),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_prefers_env_then_account() {
        let account = || Some("/opt/homebrew/bin/bash".to_string());
        assert_eq!(
            pick_shell(Some("/bin/zsh".into()), account).as_deref(),
            Some("/bin/zsh")
        );
        assert_eq!(
            pick_shell(Some(String::new()), account).as_deref(),
            Some("/opt/homebrew/bin/bash")
        );
        assert_eq!(
            pick_shell(None, account).as_deref(),
            Some("/opt/homebrew/bin/bash")
        );
        assert_eq!(pick_shell(None, || None), None);
    }

    #[test]
    fn account_shell_is_an_absolute_path() {
        // Every test machine's account has a shell.
        let s = account_shell().expect("account shell");
        assert!(s.starts_with('/'), "{s}");
    }

    #[test]
    fn locale_only_when_none_is_set() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert_eq!(locale_override(env(&[])), Some(("LC_CTYPE", "UTF-8")));
        assert_eq!(
            locale_override(env(&[("LANG", "")])),
            Some(("LC_CTYPE", "UTF-8"))
        );
        assert_eq!(locale_override(env(&[("LANG", "cs_CZ.UTF-8")])), None);
        assert_eq!(locale_override(env(&[("LC_CTYPE", "UTF-8")])), None);
        assert_eq!(locale_override(env(&[("LC_ALL", "C")])), None);
    }

    #[test]
    fn start_dir_only_from_root() {
        let home = Some(PathBuf::from("/Users/me"));
        assert_eq!(start_dir(Some(Path::new("/")), home.clone()), home);
        assert_eq!(start_dir(Some(Path::new("/tmp")), home.clone()), None);
        assert_eq!(start_dir(None, home), None);
        assert_eq!(start_dir(Some(Path::new("/")), None), None);
    }
}
