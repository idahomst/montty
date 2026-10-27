//! The config watcher: an editor's rename-save fires it; reads and unrelated files in a
//! symlink target's dir don't.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use montty::config::watch::{Scope, Watcher};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A fresh temp dir named with the pid and a counter, removed on drop.
struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Tmp {
        let p = std::env::temp_dir().join(format!(
            "montty-watch-it-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn watcher(dirs: &[PathBuf]) -> (Watcher, Receiver<()>) {
    let (tx, rx) = mpsc::channel();
    let mut w = Watcher::start(move || {
        let _ = tx.send(());
    })
    .unwrap();
    let all: Vec<_> = dirs.iter().map(|d| (d.clone(), Scope::All)).collect();
    assert!(w.set_watched(&all).is_empty());
    (w, rx)
}

fn fired(rx: &Receiver<()>, within: Duration) -> bool {
    rx.recv_timeout(within).is_ok()
}

/// Quiet time that must pass with no callback.
const QUIET: Duration = Duration::from_millis(300);
const FIRE: Duration = Duration::from_secs(2);

#[test]
fn watcher_fires_on_rename_save() {
    let t = Tmp::new();
    // Written before the watcher starts: only the rename can fire it.
    fs::write(t.0.join("a.tmp"), "x = 1\n").unwrap();
    let (_w, rx) = watcher(std::slice::from_ref(&t.0));
    fs::rename(t.0.join("a.tmp"), t.0.join("a.toml")).unwrap();
    assert!(fired(&rx, FIRE), "no change event within 2 s");
}

#[test]
fn watcher_ignores_reads() {
    let t = Tmp::new();
    fs::write(t.0.join("a.toml"), "x = 1\n").unwrap();
    let (_w, rx) = watcher(std::slice::from_ref(&t.0));
    // A reload reads every file and lists the dir: that must not fire (no reload loop).
    let _ = fs::read_to_string(t.0.join("a.toml")).unwrap();
    let _ = fs::read_dir(&t.0).unwrap().count();
    assert!(!fired(&rx, QUIET), "a read fired the watcher");
}

#[test]
fn symlink_target_dir_sees_only_targets() {
    let t = Tmp::new();
    let target = t.0.join("config.toml");
    fs::write(&target, "").unwrap();
    let (mut w, rx) = watcher(&[]);
    assert!(
        w.set_watched(&[(t.0.clone(), Scope::Only(vec![target.clone()]))])
            .is_empty()
    );
    // An unrelated file in the same dir (think $HOME/.bash_history).
    fs::write(t.0.join("unrelated"), "x").unwrap();
    fs::write(t.0.join("unrelated"), "y").unwrap();
    assert!(!fired(&rx, QUIET), "an unrelated file fired the watcher");
    fs::write(&target, "dim_amount = 0.5\n").unwrap();
    assert!(fired(&rx, FIRE), "a write to the target didn't fire");
    while fired(&rx, QUIET) {}
    // A rename-save over the target.
    fs::write(t.0.join(".config.toml.tmp"), "dim_amount = 0.6\n").unwrap();
    assert!(!fired(&rx, QUIET), "the temp file fired the watcher");
    fs::rename(t.0.join(".config.toml.tmp"), &target).unwrap();
    assert!(fired(&rx, FIRE), "a rename onto the target didn't fire");
}

#[test]
fn parent_dir_sees_only_config_dir_creation() {
    let t = Tmp::new();
    let dir = t.0.join("montty");
    let (mut w, rx) = watcher(&[]);
    assert!(
        w.set_watched(&[(t.0.clone(), Scope::Only(vec![dir.clone()]))])
            .is_empty()
    );
    fs::create_dir(t.0.join("other-app")).unwrap();
    fs::write(t.0.join("other-app.conf"), "x").unwrap();
    assert!(!fired(&rx, QUIET), "another app's file fired the watcher");
    fs::create_dir(&dir).unwrap();
    assert!(fired(&rx, FIRE), "creating the config dir didn't fire");
}

#[test]
fn set_watched_unwatches_dropped_dirs() {
    let a = Tmp::new();
    let b = Tmp::new();
    let (mut w, rx) = watcher(&[]);
    let both = [(a.0.clone(), Scope::All), (b.0.clone(), Scope::All)];
    assert!(w.set_watched(&both).is_empty());
    fs::write(a.0.join("x"), "1").unwrap();
    assert!(fired(&rx, FIRE));
    while fired(&rx, QUIET) {}
    // `a` is no longer needed.
    assert!(w.set_watched(&both[1..]).is_empty());
    fs::write(a.0.join("x"), "2").unwrap();
    assert!(!fired(&rx, QUIET), "an unwatched dir fired");
    fs::write(b.0.join("x"), "1").unwrap();
    assert!(fired(&rx, FIRE));
}

#[test]
fn failing_dir_is_reported_once_until_it_recovers() {
    let t = Tmp::new();
    let gone = t.0.join("gone");
    let (mut w, _rx) = watcher(&[]);
    let dirs = [(gone.clone(), Scope::All), (t.0.clone(), Scope::All)];
    let first = w.set_watched(&dirs);
    assert_eq!(first.len(), 1, "{first:?}");
    assert!(
        first[0].starts_with(&gone.display().to_string()),
        "{first:?}"
    );
    // Still failing on the next call: not reported again.
    assert!(w.set_watched(&dirs).is_empty());
    // It appears, then goes away again: reported again.
    fs::create_dir(&gone).unwrap();
    assert!(w.set_watched(&dirs).is_empty());
    fs::remove_dir(&gone).unwrap();
    assert_eq!(w.set_watched(&dirs).len(), 1);
}
