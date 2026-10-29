# Vendored alacritty_terminal 0.26.0

This is alacritty_terminal 0.26.0 from crates.io with one change, in `src/term/mod.rs`
(`Term::push_keyboard_mode`): when the keyboard mode stack is full, the oldest entry is removed
from `keyboard_mode_stack`. Upstream 0.26.0 removed it from `title_stack`, which panics when that
stack is empty, so 4097 pushes of `ESC[>1u` from any pane aborted montty (release builds abort
on panic). Upstream fix: alacritty commit 542147067 ("Fix Kitty keyboard mode stack size limit").

Remove this directory and the `[patch.crates-io]` entry in `Cargo.toml` once a release that
contains the fix is on crates.io. Also dropped: `Cargo.lock`, `.cargo_vcs_info.json`, `Cargo.toml.orig` and `tests/` (46 MB of fixtures; the `[[test]]` entry is removed).
