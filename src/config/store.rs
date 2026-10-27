//! The config store: `config.toml`, `profiles/*.toml` and `schemes/*.json` loaded from one
//! directory, with the last good version of each file kept across reloads.

use std::collections::BTreeMap;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::ConfigError;
use super::global::GlobalConfig;
use super::profile::{DEFAULT_PROFILE_TOML, Profile};
use super::scheme::{Scheme, builtin_schemes};
use crate::term::colors::{Palette, tango_default};

/// A profile together with its resolved color palette.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub profile: Profile,
    pub palette: Palette,
}

/// What a reload changed. `changed` lists profile stems whose `Resolved` differs, including
/// through an edited scheme.
#[derive(Debug, Default, PartialEq)]
pub struct ReloadReport {
    pub changed: Vec<String>,
    pub removed: Vec<String>,
    pub added: Vec<String>,
    pub global_changed: bool,
    /// A config file's text changed since the last reload (also when it doesn't parse,
    /// so its last good version stays), or a file appeared or went away.
    pub edited: bool,
    pub errors: Vec<ConfigError>,
}

/// The last good parse of a profile file, and the line of its `scheme` key.
struct ProfileEntry {
    profile: Profile,
    scheme_line: Option<usize>,
}

pub struct Store {
    dir: PathBuf,
    home: Option<PathBuf>,
    global: GlobalConfig,
    /// Built-in schemes, then user schemes on top (same stem replaces).
    schemes: BTreeMap<String, Scheme>,
    user_schemes: BTreeMap<String, Scheme>,
    profiles: BTreeMap<String, ProfileEntry>,
    resolved: BTreeMap<String, Arc<Resolved>>,
    /// The default profile's stem (see `default_stem`), kept with `resolved`.
    default: String,
    /// `resolved` and `default` as they were before the last reload.
    previous: BTreeMap<String, Arc<Resolved>>,
    previous_default: String,
    fallback: Arc<Resolved>,
    /// The text of the files read by the last reload (see `ReloadReport::edited`).
    sources: u64,
    /// The text of the files the running reload has read so far.
    reading: DefaultHasher,
}

/// Files in `dir` with the extension `ext` as `(stem, path)`, sorted by stem. Hidden files
/// and editor leftovers (`foo.toml~`, `#foo.toml#`, `.foo.toml.swp`) don't count. A missing
/// directory is empty.
fn list_files(dir: &Path, ext: &str) -> Result<Vec<(String, PathBuf)>, std::io::Error> {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if absent(&e) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for entry in rd {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') || name.starts_with('#') {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some(ext) || !path.is_file() {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            out.push((stem.to_string(), path.clone()));
        }
    }
    out.sort();
    Ok(out)
}

/// `stem` in `map`, else `default` in it.
fn lookup<'a>(
    map: &'a BTreeMap<String, Arc<Resolved>>,
    default: &str,
    stem: &str,
) -> Option<&'a Arc<Resolved>> {
    map.get(stem).or_else(|| map.get(default))
}

fn absent(e: &std::io::Error) -> bool {
    matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory)
}

fn io_error(rel: &Path, e: &std::io::Error) -> ConfigError {
    ConfigError {
        file: rel.to_path_buf(),
        line: None,
        msg: e.to_string(),
    }
}

/// The 1-based line of the top-level `scheme` key in `src`.
fn scheme_key_line(src: &str) -> Option<usize> {
    src.lines()
        .position(|l| {
            l.trim_start()
                .strip_prefix("scheme")
                .is_some_and(|r| r.trim_start().starts_with('='))
        })
        .map(|i| i + 1)
}

impl Store {
    /// Loads the config directory, writing `profiles/default.toml` first when `profiles/`
    /// has no profile. Never fails: problems come back as errors and the store runs on
    /// built-in defaults for whatever could not be loaded.
    #[cfg(test)]
    pub fn load(dir: &Path, home: Option<&Path>) -> (Store, Vec<ConfigError>) {
        let (store, first_run, errors) = Store::load_split(dir, home);
        (store, first_run.into_iter().chain(errors).collect())
    }

    /// `load`, with the first-run write's error apart from the files' errors: a reload can
    /// fix the files, not the write.
    pub fn load_split(
        dir: &Path,
        home: Option<&Path>,
    ) -> (Store, Option<ConfigError>, Vec<ConfigError>) {
        let mut store = Store::empty(dir, home);
        let first_run = store.write_default_profile();
        let errors = store.reload().errors;
        (store, first_run, errors)
    }

    /// Built-in defaults without a config directory (`$HOME` unset): nothing is read or
    /// written, and `reload` changes nothing.
    pub fn builtin() -> Store {
        Store::empty(Path::new(""), None)
    }

    fn empty(dir: &Path, home: Option<&Path>) -> Store {
        Store {
            dir: dir.to_path_buf(),
            home: home.map(Path::to_path_buf),
            global: GlobalConfig::default(),
            schemes: BTreeMap::new(),
            user_schemes: BTreeMap::new(),
            profiles: BTreeMap::new(),
            resolved: BTreeMap::new(),
            default: "default".to_string(),
            previous: BTreeMap::new(),
            previous_default: "default".to_string(),
            fallback: Arc::new(Resolved {
                profile: Profile::default(),
                palette: tango_default(),
            }),
            sources: 0,
            reading: DefaultHasher::new(),
        }
    }

    /// First run: no `*.toml` in `profiles/`.
    fn write_default_profile(&self) -> Option<ConfigError> {
        let profiles = self.dir.join("profiles");
        match list_files(&profiles, "toml") {
            Ok(files) if files.is_empty() => {}
            _ => return None,
        }
        let rel = Path::new("profiles").join("default.toml");
        // `create_new` fails on any existing entry, even a dangling symlink, so a racing
        // writer or a link is never written through.
        fs::create_dir_all(&profiles)
            .and_then(|()| {
                let mut f = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(profiles.join("default.toml"))?;
                f.write_all(DEFAULT_PROFILE_TOML.as_bytes())
            })
            .err()
            .filter(|e| e.kind() != ErrorKind::AlreadyExists)
            .map(|e| io_error(&rel, &e))
    }

    /// Re-parses every file. A file that fails to parse keeps its last good version; a file
    /// that is gone is dropped.
    pub fn reload(&mut self) -> ReloadReport {
        // `builtin`: an empty path would read the current directory.
        if self.dir.as_os_str().is_empty() {
            return ReloadReport::default();
        }
        let mut errors = Vec::new();
        let old_global = self.global.clone();
        self.previous = std::mem::take(&mut self.resolved);
        self.previous_default = std::mem::take(&mut self.default);
        self.reading = DefaultHasher::new();

        self.load_global(&mut errors);
        self.load_schemes(&mut errors);
        self.load_profiles(&mut errors);
        self.resolve(&mut errors);
        self.default = self.find_default();
        // A `default_profile` naming no profile still falls back (spec), but not silently.
        if let Some(line) = self.global.default_profile_line
            && !self.resolved.contains_key(&self.global.default_profile)
        {
            errors.push(ConfigError {
                file: PathBuf::from("config.toml"),
                line: Some(line),
                msg: format!(
                    "no profile '{}' (using '{}')",
                    self.global.default_profile, self.default
                ),
            });
        }

        let sources = self.reading.finish();
        let mut report = ReloadReport {
            global_changed: self.global != old_global,
            edited: sources != std::mem::replace(&mut self.sources, sources),
            ..ReloadReport::default()
        };
        for (stem, new) in &self.resolved {
            match self.previous.get(stem) {
                None => report.added.push(stem.clone()),
                Some(old) if old != new => report.changed.push(stem.clone()),
                Some(_) => {}
            }
        }
        report.removed = self
            .previous
            .keys()
            .filter(|s| !self.resolved.contains_key(*s))
            .cloned()
            .collect();
        report.errors = errors;
        report
    }

    /// Read config file `path`, noting its text for `ReloadReport::edited`.
    fn read(&mut self, path: &Path) -> std::io::Result<String> {
        let src = fs::read_to_string(path)?;
        path.hash(&mut self.reading);
        src.hash(&mut self.reading);
        Ok(src)
    }

    fn load_global(&mut self, errors: &mut Vec<ConfigError>) {
        let rel = PathBuf::from("config.toml");
        match self.read(&self.dir.join(&rel)) {
            Ok(src) => match GlobalConfig::parse(&src, &rel) {
                Ok(g) => self.global = g,
                Err(e) => errors.push(e),
            },
            Err(e) if absent(&e) => self.global = GlobalConfig::default(),
            Err(e) => errors.push(io_error(&rel, &e)),
        }
    }

    fn load_schemes(&mut self, errors: &mut Vec<ConfigError>) {
        let rel_dir = Path::new("schemes");
        match list_files(&self.dir.join(rel_dir), "json") {
            Ok(files) => {
                let mut seen = Vec::new();
                for (stem, path) in files {
                    let rel = rel_dir.join(path.file_name().unwrap_or_default());
                    match self.read(&path) {
                        Ok(src) => match Scheme::parse(&src, &rel) {
                            Ok(s) => {
                                self.user_schemes.insert(stem.clone(), s);
                            }
                            Err(e) => errors.push(e),
                        },
                        Err(e) => errors.push(io_error(&rel, &e)),
                    }
                    seen.push(stem);
                }
                self.user_schemes.retain(|k, _| seen.contains(k));
            }
            Err(e) => errors.push(io_error(rel_dir, &e)),
        }
        self.schemes = builtin_schemes()
            .into_iter()
            .map(|(stem, s)| (stem.to_string(), s))
            .chain(self.user_schemes.clone())
            .collect();
    }

    fn load_profiles(&mut self, errors: &mut Vec<ConfigError>) {
        let rel_dir = Path::new("profiles");
        match list_files(&self.dir.join(rel_dir), "toml") {
            Ok(files) => {
                let mut seen = Vec::new();
                for (stem, path) in files {
                    let rel = rel_dir.join(path.file_name().unwrap_or_default());
                    match self.read(&path) {
                        Ok(src) => match Profile::parse(&src, &rel, self.home.as_deref()) {
                            Ok(profile) => {
                                let scheme_line = scheme_key_line(&src);
                                self.profiles.insert(
                                    stem.clone(),
                                    ProfileEntry {
                                        profile,
                                        scheme_line,
                                    },
                                );
                            }
                            Err(e) => errors.push(e),
                        },
                        Err(e) => errors.push(io_error(&rel, &e)),
                    }
                    seen.push(stem);
                }
                self.profiles.retain(|k, _| seen.contains(k));
            }
            Err(e) => errors.push(io_error(rel_dir, &e)),
        }
    }

    /// A scheme by file stem, else by display name (case-insensitive).
    fn find_scheme(&self, key: &str) -> Option<Palette> {
        self.schemes
            .get(key)
            .or_else(|| {
                self.schemes
                    .values()
                    .find(|s| s.name.eq_ignore_ascii_case(key))
            })
            .map(|s| s.palette.clone())
    }

    fn resolve(&mut self, errors: &mut Vec<ConfigError>) {
        let mut out = BTreeMap::new();
        for (stem, entry) in &self.profiles {
            let palette = match entry.profile.palette(&|k| self.find_scheme(k)) {
                Ok(p) => p,
                Err(msg) => {
                    errors.push(ConfigError {
                        file: Path::new("profiles").join(format!("{stem}.toml")),
                        line: entry.scheme_line,
                        msg,
                    });
                    // Unknown scheme: Tango, with the profile's own color overrides.
                    entry
                        .profile
                        .palette(&|_| Some(tango_default()))
                        .unwrap_or_else(|_| tango_default())
                }
            };
            out.insert(
                stem.clone(),
                Arc::new(Resolved {
                    profile: entry.profile.clone(),
                    palette,
                }),
            );
        }
        self.resolved = out;
    }

    pub fn global(&self) -> &GlobalConfig {
        &self.global
    }

    /// The profile for `stem`; an unknown stem gets the default profile, and with no
    /// profiles at all the built-in defaults. Lookups only, no allocation.
    pub fn profile(&self, stem: &str) -> Arc<Resolved> {
        lookup(&self.resolved, &self.default, stem)
            .unwrap_or(&self.fallback)
            .clone()
    }

    /// `profile(stem)` as it was before the last reload (what panes ran under until now).
    pub fn previous_profile(&self, stem: &str) -> Arc<Resolved> {
        lookup(&self.previous, &self.previous_default, stem)
            .unwrap_or(&self.fallback)
            .clone()
    }

    /// Whether profile `stem` exists (else it resolves to the default).
    pub fn contains(&self, stem: &str) -> bool {
        self.resolved.contains_key(stem)
    }

    /// `default_profile` from `config.toml`, else `default`, else the alphabetically first
    /// profile, else `default`. Computed on each (re)load.
    pub fn default_stem(&self) -> &str {
        &self.default
    }

    /// `default_stem` before the last reload.
    pub fn previous_default_stem(&self) -> &str {
        &self.previous_default
    }

    fn find_default(&self) -> String {
        [self.global.default_profile.as_str(), "default"]
            .into_iter()
            .find(|s| self.resolved.contains_key(*s))
            .map(str::to_string)
            .or_else(|| self.resolved.keys().next().cloned())
            .unwrap_or_else(|| "default".to_string())
    }

    /// `(stem, display name)` of every profile, sorted by display name then stem.
    pub fn stems(&self) -> Vec<(String, String)> {
        let mut v: Vec<_> = self
            .resolved
            .iter()
            .map(|(stem, r)| {
                let name = match r.profile.name.as_str() {
                    "" => stem.clone(),
                    n => n.to_string(),
                };
                (stem.clone(), name)
            })
            .collect();
        v.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        v
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Tmp {
            let p = std::env::temp_dir().join(format!(
                "montty-store-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }

        fn write(&self, rel: &str, content: &str) {
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, content).unwrap();
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const SCHEME: &str = r##"{"name":"My","comment":"","foreground-color":"#111111",
        "background-color":"#222222","palette":["#000000","#000000","#000000","#000000",
        "#000000","#000000","#000000","#000000","#000000","#000000","#000000","#000000",
        "#000000","#000000","#000000","#000000"]}"##;

    #[test]
    fn first_run_writes_default() {
        let t = Tmp::new();
        let (s, errs) = Store::load(&t.0, None);
        assert!(errs.is_empty(), "{errs:?}");
        let written = fs::read_to_string(t.0.join("profiles/default.toml")).unwrap();
        assert_eq!(written, DEFAULT_PROFILE_TOML);
        assert_eq!(s.profile("default").profile.name, "Default");
    }

    #[test]
    fn existing_profiles_not_overwritten() {
        let t = Tmp::new();
        t.write("profiles/work.toml", "name = \"Work\"\n");
        let (s, errs) = Store::load(&t.0, None);
        assert!(errs.is_empty(), "{errs:?}");
        assert!(!t.0.join("profiles/default.toml").exists());
        assert_eq!(s.default_stem(), "work");
        assert_eq!(s.stems(), vec![("work".to_string(), "Work".to_string())]);
    }

    #[test]
    fn reload_reports_changed() {
        let t = Tmp::new();
        let (mut s, _) = Store::load(&t.0, None);
        t.write("profiles/default.toml", "font_size = 14\n");
        let r = s.reload();
        assert_eq!(r.changed, vec!["default"]);
        assert!(r.errors.is_empty());
        assert_eq!(s.profile("default").profile.font_size, 14.0);
        assert_eq!(s.reload(), ReloadReport::default());
    }

    #[test]
    fn reload_reports_edits_of_broken_files() {
        let t = Tmp::new();
        t.write("profiles/work.toml", "x\n");
        let (mut s, errors) = Store::load(&t.0, None);
        assert_eq!(errors.len(), 1);
        // Saved again with the same error: no profile changed, but the text did.
        t.write("profiles/work.toml", "x\nshell = \"/bin/nope\"\n");
        let r = s.reload();
        assert!(r.edited);
        assert!(r.changed.is_empty());
        assert_eq!(r.errors, errors);
        assert!(!s.reload().edited);
        // A removed file is an edit too.
        fs::remove_file(t.0.join("profiles/work.toml")).unwrap();
        assert!(s.reload().edited);
    }

    #[test]
    fn scheme_change_marks_users() {
        let t = Tmp::new();
        t.write("schemes/my.json", SCHEME);
        t.write("profiles/default.toml", "scheme = \"my\"\n");
        t.write("profiles/other.toml", "");
        let (mut s, errs) = Store::load(&t.0, None);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(s.profile("default").palette.fg.r, 0x11);
        t.write("schemes/my.json", &SCHEME.replace("#111111", "#333333"));
        let r = s.reload();
        assert_eq!(r.changed, vec!["default"]);
        assert_eq!(s.profile("default").palette.fg.r, 0x33);
    }

    #[test]
    fn scheme_found_by_display_name() {
        let t = Tmp::new();
        t.write("schemes/my.json", SCHEME);
        t.write("profiles/default.toml", "scheme = \"MY\"\n");
        let (s, errs) = Store::load(&t.0, None);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(s.profile("default").palette.fg.r, 0x11);
    }

    #[test]
    fn unknown_scheme_falls_back_to_tango() {
        let t = Tmp::new();
        t.write(
            "profiles/default.toml",
            "font_size = 13\nscheme = \"nope\"\n",
        );
        let (s, errs) = Store::load(&t.0, None);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].file, Path::new("profiles/default.toml"));
        assert_eq!(errs[0].line, Some(2));
        let r = s.profile("default");
        assert_eq!(r.palette, tango_default());
        assert_eq!(r.profile.font_size, 13.0);
    }

    #[test]
    fn reload_keeps_last_good_on_parse_error() {
        let t = Tmp::new();
        let (mut s, _) = Store::load(&t.0, None);
        let before = s.profile("default").profile.font_size;
        t.write("profiles/default.toml", "font_size = ");
        let r = s.reload();
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.errors[0].file, Path::new("profiles/default.toml"));
        assert!(r.errors[0].line.is_some());
        assert!(r.changed.is_empty());
        assert_eq!(s.profile("default").profile.font_size, before);
    }

    #[test]
    fn reload_recovers_after_fix() {
        let t = Tmp::new();
        let (mut s, _) = Store::load(&t.0, None);
        t.write("profiles/default.toml", "font_size = ");
        s.reload();
        t.write("profiles/default.toml", "font_size = 15\n");
        let r = s.reload();
        assert_eq!(r.changed, vec!["default"]);
        assert!(r.errors.is_empty());
    }

    #[test]
    fn removed_profile_reported() {
        let t = Tmp::new();
        t.write("profiles/default.toml", "");
        t.write("profiles/work.toml", "font_size = 16\n");
        let (mut s, _) = Store::load(&t.0, None);
        fs::remove_file(t.0.join("profiles/work.toml")).unwrap();
        let r = s.reload();
        assert_eq!(r.removed, vec!["work"]);
        assert_eq!(s.profile("work").profile.font_size, 11.0);
        t.write("profiles/work.toml", "");
        assert_eq!(s.reload().added, vec!["work"]);
    }

    #[test]
    fn editor_leftovers_ignored() {
        let t = Tmp::new();
        t.write("profiles/default.toml", "");
        let (mut s, _) = Store::load(&t.0, None);
        for f in [
            ".a.toml.swp",
            "a.toml~",
            "#a.toml#",
            ".a.toml",
            "a.toml.bak",
        ] {
            t.write(&format!("profiles/{f}"), "garbage = ");
        }
        t.write("schemes/.x.json", "garbage");
        assert_eq!(s.reload(), ReloadReport::default());
    }

    #[test]
    fn global_changes_reported() {
        let t = Tmp::new();
        let (mut s, _) = Store::load(&t.0, None);
        t.write("config.toml", "default_profile = \"x\"\n");
        let r = s.reload();
        assert!(r.global_changed);
        assert_eq!(s.global().default_profile, "x");
    }

    #[test]
    fn unreadable_dir_runs_on_defaults() {
        let t = Tmp::new();
        t.write("file", "");
        let (s, errs) = Store::load(&t.0.join("file"), None);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert_eq!(s.profile("default").profile, Profile::default());
        assert!(s.stems().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn first_run_error_kept_apart() {
        use std::os::unix::fs::PermissionsExt;
        // root writes anyway.
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        // A read-only `profiles/`: the default profile can't be written, and no reload
        // fixes that; a broken config.toml is a file error.
        let t = Tmp::new();
        t.write("config.toml", "dim_amount = 9\n");
        fs::create_dir(t.0.join("profiles")).unwrap();
        fs::set_permissions(t.0.join("profiles"), fs::Permissions::from_mode(0o555)).unwrap();
        let (_s, first_run, errs) = Store::load_split(&t.0, None);
        let first_run = first_run.expect("first-run write error");
        assert_eq!(first_run.file, Path::new("profiles/default.toml"));
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert_eq!(errs[0].file, Path::new("config.toml"));
        // `load` reports both, the write first.
        let (_s, all) = Store::load(&t.0, None);
        assert_eq!(all, [vec![first_run], errs].concat());
    }

    #[test]
    fn missing_default_profile_falls_back() {
        let t = Tmp::new();
        t.write("config.toml", "default_profile = \"missing\"\n");
        t.write("profiles/default.toml", "");
        t.write("profiles/work.toml", "");
        let (mut s, errs) = Store::load(&t.0, None);
        assert_eq!(s.default_stem(), "default");
        // A typo is visible, on its line; the fallback is unchanged.
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert_eq!(
            errs[0].to_string(),
            "config.toml:1: no profile 'missing' (using 'default')"
        );
        // The profile appears: the problem is gone and the default is the new profile.
        t.write("profiles/missing.toml", "");
        let r = s.reload();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(s.default_stem(), "missing");
        // Fixing the name does too.
        t.write("config.toml", "default_profile = \"work\"\n");
        std::fs::remove_file(t.0.join("profiles/missing.toml")).unwrap();
        assert!(s.reload().errors.is_empty());
        assert_eq!(s.default_stem(), "work");
    }

    #[test]
    fn unset_default_profile_falls_back_silently() {
        // `default.toml` is gone and `default_profile` isn't set: nothing to report.
        let t = Tmp::new();
        t.write("profiles/work.toml", "");
        let (s, errs) = Store::load(&t.0, None);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(s.default_stem(), "work");
    }

    #[test]
    fn invalid_global_keeps_chain_working() {
        let t = Tmp::new();
        t.write("config.toml", "default_profile = ");
        t.write("profiles/work.toml", "");
        let (s, errs) = Store::load(&t.0, None);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert_eq!(errs[0].file, Path::new("config.toml"));
        assert_eq!(s.default_stem(), "work");
    }

    #[cfg(unix)]
    #[test]
    fn first_run_does_not_write_through_dangling_symlink() {
        let t = Tmp::new();
        let target = t.0.join("outside.toml");
        fs::create_dir_all(t.0.join("cfg/profiles")).unwrap();
        std::os::unix::fs::symlink(&target, t.0.join("cfg/profiles/default.toml")).unwrap();
        let (_s, errs) = Store::load(&t.0.join("cfg"), None);
        assert!(errs.is_empty(), "{errs:?}");
        assert!(!target.exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_profile_and_dir_load_and_reload() {
        let t = Tmp::new();
        t.write("real/profiles/default.toml", "");
        t.write("outside.toml", "name = \"Out\"\n");
        let real = t.0.join("real");
        std::os::unix::fs::symlink(t.0.join("outside.toml"), real.join("profiles/linked.toml"))
            .unwrap();
        std::os::unix::fs::symlink(&real, t.0.join("link")).unwrap();
        let (mut s, errs) = Store::load(&t.0.join("link"), None);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(s.profile("linked").profile.name, "Out");
        t.write("outside.toml", "name = \"Out2\"\n");
        assert_eq!(s.reload().changed, vec!["linked"]);
    }

    #[test]
    fn default_stem_follows_reload() {
        let t = Tmp::new();
        t.write("profiles/default.toml", "");
        t.write("profiles/work.toml", "font_size = 16\n");
        let (mut s, _) = Store::load(&t.0, None);
        let d: &str = s.default_stem();
        assert_eq!(d, "default");
        t.write("config.toml", "default_profile = \"work\"\n");
        s.reload();
        assert_eq!(s.default_stem(), "work");
        // An unknown stem resolves to the cached default.
        assert_eq!(s.profile("nope").profile.font_size, 16.0);
        fs::remove_file(t.0.join("profiles/work.toml")).unwrap();
        s.reload();
        assert_eq!(s.default_stem(), "default");
        assert_eq!(s.profile("nope").profile.font_size, 11.0);
    }

    #[test]
    fn previous_profile_is_before_reload() {
        let t = Tmp::new();
        t.write("profiles/default.toml", "font_size = 12\n");
        t.write("profiles/work.toml", "font_size = 16\n");
        let (mut s, _) = Store::load(&t.0, None);
        t.write("profiles/default.toml", "font_size = 13\n");
        fs::remove_file(t.0.join("profiles/work.toml")).unwrap();
        s.reload();
        assert_eq!(s.previous_profile("default").profile.font_size, 12.0);
        assert_eq!(s.profile("default").profile.font_size, 13.0);
        // `work` was there before the reload, and falls back to the default now.
        assert_eq!(s.previous_profile("work").profile.font_size, 16.0);
        assert_eq!(s.profile("work").profile.font_size, 13.0);
        // Unknown before: the default then.
        assert_eq!(s.previous_profile("nope").profile.font_size, 12.0);
        assert!(s.contains("default") && !s.contains("work"));
        t.write("config.toml", "default_profile = \"x\"\n");
        s.reload();
        assert_eq!(s.previous_default_stem(), "default");
        assert_eq!(s.previous_profile("default").profile.font_size, 13.0);
    }

    #[test]
    fn builtin_store_is_defaults_and_never_reloads() {
        let mut s = Store::builtin();
        assert_eq!(s.global(), &GlobalConfig::default());
        assert_eq!(s.profile("default").palette, tango_default());
        assert_eq!(s.reload(), ReloadReport::default());
        assert!(s.stems().is_empty());
    }
}
