# Vendored alacritty_terminal 0.26.0

This is alacritty_terminal 0.26.0 from crates.io with three upstream fixes that no release has yet
(checked 2026-11-10: 0.26.0 is the newest on crates.io).

1. In `src/term/mod.rs` (`Term::push_keyboard_mode`): when the keyboard mode stack is full, the oldest entry is removed
   from `keyboard_mode_stack`. Upstream 0.26.0 removed it from `title_stack`, which panics when that
   stack is empty, so 4097 pushes of `ESC[>1u` from any pane aborted montty (release builds abort
   on panic). Upstream fix: alacritty commit 542147067 ("Fix Kitty keyboard mode stack size limit").
2. In `src/term/cell.rs`: a cell keeps at most 9 zero-width characters (`ArrayVec` instead of `Vec`; adds the
   `arrayvec` dependency to `Cargo.toml`, a crate already in montty's tree). Upstream 0.26.0 kept every one, so output
   could grow a single cell's memory without bound. Upstream fix: alacritty commit ede2ac144 ("Fix unbounded zerowidth
   character limit").
3. In `src/tty/unix.rs`: a PTY that can't be set non-blocking is an error from `tty::new` (montty shows it as a spawn
   failure) instead of a failed assertion, which aborted montty. Upstream fix: alacritty commit 985394dd4 ("Handle PTY
   non-blocking failure gracefully").

Remove this directory and the `[patch.crates-io]` entry in `Cargo.toml` once a release that
contains all three fixes is on crates.io. Also dropped: `Cargo.lock`, `.cargo_vcs_info.json`, `Cargo.toml.orig`, `README.md`,
`CHANGELOG.md` (and the `readme` key) and `tests/` (46 MB of fixtures; the `[[test]]` entry is removed).
