use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::TermMode;
use montty::term::pane::{Pane, PaneOptions, UserEvent};

fn size(cols: u16, lines: u16) -> WindowSize {
    WindowSize {
        num_lines: lines,
        num_cols: cols,
        cell_width: 8,
        cell_height: 16,
    }
}

fn opts(cmd: &[&str]) -> PaneOptions {
    PaneOptions {
        command: Some(cmd.iter().map(|s| s.to_string()).collect()),
        runs_command: true,
        working_dir: None,
        scrollback: 100,
        cursor: Default::default(),
        window_id: None,
    }
}

/// Wait up to 5 s for line 0 of `pane` to read `want` (trailing blanks trimmed);
/// returns the line as last seen.
fn wait_line0(pane: &Pane, rx: &Receiver<UserEvent>, want: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut line = String::new();
    while Instant::now() < deadline {
        wait_for(rx, |e| matches!(e, Event::Wakeup));
        let term = pane.term.lock();
        line = (0..term.columns())
            .map(|c| term.grid()[Line(0)][Column(c)].c)
            .collect::<String>()
            .trim_end()
            .to_string();
        if line == want {
            break;
        }
    }
    line
}

/// Wait up to 5 s for an event matching `pred`.
fn wait_for(rx: &Receiver<UserEvent>, mut pred: impl FnMut(&Event) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(UserEvent::Term(_, ev)) if pred(&ev) => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}

#[test]
fn spawn_runs_command_and_output_reaches_grid() {
    let (tx, rx) = mpsc::channel();
    let pane = Pane::spawn(
        1,
        &opts(&["sh", "-c", "printf hello; sleep 0.2"]),
        size(20, 5),
        Arc::new(tx),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut ok = false;
    while !ok && Instant::now() < deadline {
        wait_for(&rx, |e| matches!(e, Event::Wakeup));
        let term = pane.term.lock();
        let line: String = (0..5).map(|c| term.grid()[Line(0)][Column(c)].c).collect();
        ok = line == "hello";
    }
    assert!(ok, "line 0 never started with hello");
}

#[test]
fn child_exit_event_delivered() {
    let (tx, rx) = mpsc::channel();
    let _pane = Pane::spawn(2, &opts(&["sh", "-c", "exit 3"]), size(20, 5), Arc::new(tx)).unwrap();
    assert!(wait_for(&rx, |e| matches!(
        e,
        Event::ChildExit(_) | Event::Exit
    )));
}

#[test]
fn spawn_nonexistent_command_errors() {
    let (tx, rx) = mpsc::channel();
    match Pane::spawn(3, &opts(&["/nonexistent/prog"]), size(20, 5), Arc::new(tx)) {
        Err(_) => {}
        Ok(_pane) => {
            assert!(wait_for(&rx, |e| matches!(
                e,
                Event::ChildExit(_) | Event::Exit
            )));
        }
    }
}

#[test]
fn resize_changes_grid() {
    let (tx, _rx) = mpsc::channel();
    let mut pane = Pane::spawn(
        4,
        &opts(&["sh", "-c", "sleep 1"]),
        size(10, 3),
        Arc::new(tx),
    )
    .unwrap();
    pane.resize(size(20, 5));
    let term = pane.term.lock();
    assert_eq!((term.columns(), term.screen_lines()), (20, 5));
}

#[test]
fn child_env_window_ids() {
    // alacritty's tty::new exports the pane id as WINDOWID and ALACRITTY_WINDOW_ID.
    let cmd = [
        "sh",
        "-c",
        "printf '[%s][%s]' \"$WINDOWID\" \"$ALACRITTY_WINDOW_ID\"; sleep 0.5",
    ];
    let (tx, rx) = mpsc::channel();
    let pane = Pane::spawn(7, &opts(&cmd), size(40, 5), Arc::new(tx)).unwrap();
    assert_eq!(wait_line0(&pane, &rx, "[][]"), "[][]");

    let (tx, rx) = mpsc::channel();
    let with_id = PaneOptions {
        window_id: Some(4242),
        ..opts(&cmd)
    };
    let pane = Pane::spawn(8, &with_id, size(40, 5), Arc::new(tx)).unwrap();
    assert_eq!(wait_line0(&pane, &rx, "[4242][]"), "[4242][]");
}

/// Poll `f` every 20 ms for up to 5 s; true as soon as it holds.
fn eventually(mut f: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn foreground_child_detected() {
    if !std::path::Path::new("/bin/bash").exists() {
        return;
    }
    let (tx, _rx) = mpsc::channel();
    // No rc files: a user's prompt hooks may run their own foreground jobs.
    let cmd = ["/bin/bash", "--norc", "--noprofile", "-i"];
    let pane = Pane::spawn(9, &opts(&cmd), size(80, 24), Arc::new(tx)).unwrap();
    assert!(
        eventually(|| !pane.has_foreground_child()),
        "bash at its prompt"
    );
    pane.write(b"sleep 5\r".to_vec());
    assert!(eventually(|| pane.has_foreground_child()), "sleep running");
    pane.write(b"\x03".to_vec());
    assert!(
        eventually(|| !pane.has_foreground_child()),
        "back at the prompt"
    );
}

#[test]
fn foreground_child_false_for_plain_command() {
    let (tx, _rx) = mpsc::channel();
    let pane = Pane::spawn(
        10,
        &opts(&["sh", "-c", "sleep 2"]),
        size(20, 5),
        Arc::new(tx),
    )
    .unwrap();
    // A non-interactive sh has no job control: its children stay in its group.
    for _ in 0..10 {
        assert!(!pane.has_foreground_child());
        std::thread::sleep(Duration::from_millis(30));
    }
}

#[test]
fn explicit_command_is_reported() {
    let (tx, _rx) = mpsc::channel();
    let pane = Pane::spawn(11, &opts(&["sleep", "2"]), size(20, 5), Arc::new(tx)).unwrap();
    assert!(pane.runs_command());
    // The command leads its own session: tcgetpgrp can't tell it from a shell.
    assert!(!pane.has_foreground_child());
    let (tx, _rx) = mpsc::channel();
    let shell = PaneOptions {
        command: None,
        runs_command: false,
        ..opts(&[])
    };
    let pane = Pane::spawn(12, &shell, size(20, 5), Arc::new(tx)).unwrap();
    assert!(!pane.runs_command());
    // A profile's shell is spawned by argv too, but it is still a shell.
    let (tx, _rx) = mpsc::channel();
    let shell = PaneOptions {
        runs_command: false,
        ..opts(&["sh"])
    };
    let pane = Pane::spawn(13, &shell, size(20, 5), Arc::new(tx)).unwrap();
    assert!(!pane.runs_command());
}

#[test]
fn set_options_shrinks_scrollback() {
    let (tx, rx) = mpsc::channel();
    let o = PaneOptions {
        scrollback: 1000,
        ..opts(&["sh", "-c", "seq 1 500; sleep 2"])
    };
    let pane = Pane::spawn(14, &o, size(20, 5), Arc::new(tx)).unwrap();
    // 500 lines on a 5-line screen: the rest went to the history.
    let filled = eventually(|| {
        while rx.try_recv().is_ok() {}
        pane.term.lock().grid().history_size() >= 400
    });
    assert!(filled, "history {}", pane.term.lock().grid().history_size());
    pane.set_options(10, Default::default());
    assert!(pane.term.lock().grid().history_size() <= 10);
}

#[test]
fn set_options_applies_on_the_alt_screen() {
    let (tx, rx) = mpsc::channel();
    // 300 lines of history, then the alternate screen (as vim does), and stay there.
    let o = PaneOptions {
        scrollback: 1000,
        ..opts(&["sh", "-c", "seq 1 300; printf '\\033[?1049h'; sleep 3"])
    };
    let pane = Pane::spawn(15, &o, size(20, 5), Arc::new(tx)).unwrap();
    let on_alt = eventually(|| {
        while rx.try_recv().is_ok() {}
        pane.term.lock().mode().contains(TermMode::ALT_SCREEN)
    });
    assert!(on_alt, "never reached the alternate screen");
    // The alternate screen has no history; the primary grid's is parked, and shrinking the
    // scrollback must still reach it (alacritty keeps the inactive grid up to date), so
    // leaving the alternate screen shows the shorter history.
    pane.term.lock().swap_alt();
    assert!(pane.term.lock().grid().history_size() >= 200);
    pane.term.lock().swap_alt();
    pane.set_options(10, Default::default());
    pane.term.lock().swap_alt();
    assert!(pane.term.lock().grid().history_size() <= 10);
}

#[test]
fn pane_cwd_follows_shell() {
    let (tx, _rx) = mpsc::channel();
    let pane = Pane::spawn(
        20,
        &opts(&["sh", "-c", "cd /tmp && exec sleep 5"]),
        size(20, 5),
        Arc::new(tx),
    )
    .unwrap();
    let want = std::fs::canonicalize("/tmp").unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while pane.cwd().as_deref() != Some(want.as_path()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(pane.cwd(), Some(want));
}

#[test]
fn cwd_of_deleted_dir_is_none() {
    let dir = std::env::temp_dir().join(format!("montty-cwd-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (tx, _rx) = mpsc::channel();
    let cmd = format!("cd {} && exec sleep 5", dir.display());
    let pane = Pane::spawn(21, &opts(&["sh", "-c", &cmd]), size(20, 5), Arc::new(tx)).unwrap();
    let want = std::fs::canonicalize(&dir).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while pane.cwd().as_deref() != Some(want.as_path()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(pane.cwd(), Some(want), "shell never reached the dir");
    std::fs::remove_dir(&dir).unwrap();
    assert_eq!(pane.cwd(), None);
}
