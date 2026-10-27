# montty performance

Priority order is responsiveness > simplicity > features, so a few numbers are gates: a change that regresses them
should not land. This page says how to measure them and what montty 0.1.0 measures.

## Gates and targets

| Metric | Requirement | Kind |
|---|---|---|
| Key-to-screen latency | median within one frame (16.7 ms at 60 Hz) of Alacritty | gate |
| `cat` of a 100 MB file | at most about 1.5x Alacritty's time; other panes stay responsive meanwhile | gate |
| Idle CPU | about 0 %, no redraws, no timers | gate |
| Latency with a neighbouring pane flooding (`yes`) | median within one frame of the idle median | gate |
| Idle RSS, 10 panes | under about 100 MB | target |
| Stripped release binary | under about 8 MB | target |
| `cat` throughput after a change | no more than 5 % slower than before it | regression check |

## How to measure

Everything goes through `scripts/bench.sh`; run it from the repo root against a release build
(`cargo build --release`). The terminal under test is any command, for example
`scripts/bench.sh throughput target/release/montty` or `scripts/bench.sh throughput alacritty`. The per-terminal
"run a command" flag is chosen by the program's basename (montty, alacritty: `-e`; tilix: `--new-process -e`;
gnome-terminal: `--wait --`; override with `BENCH_STYLE`).

| Mode | Measures |
|---|---|
| `gen` | writes the workload, `~/.cache/montty-bench/100mb.txt`: 100 MiB, 1,226,619 lines, mixed plain and ANSI colour text |
| `throughput <term>` | times `cat` of the workload inside the terminal (`date` before and after), so it measures how fast the terminal drains the PTY, not time to final pixels. Same method for every terminal. Median of 3 runs |
| `idle <term>` | CPU ticks (utime+stime from `/proc/<pid>/stat`) over 10 s, starting 3 s after launch |
| `flood <term>` | 5 s of `yes`: CPU use, RSS and a clean exit |
| `rss10 <montty>` | opens a 10-pane session and reports RSS after 5 s idle |
| `latency [--flood] [--samples N] <term>` | key-to-screen latency, see below; `--flood` runs `yes` in a side-by-side pane |

`scripts/latency.py` (used by `bench.sh latency`) types `x` through XTest into a `cat` in the window it started and
reads the window's own contents with `XGetImage` after each key until the character appears. That is the part
terminals differ in (event handling, drawing and buffer swap); it is not photon latency through a compositor. It needs
the window to hold the input focus and aborts if it loses it.

Setup and cautions:
- Use a release build, with nothing else running. Compare against other terminals at the same font and grid size
  (montty's default window is 80x24 cells; match the others to it).
- Use an empty `XDG_CONFIG_HOME` so your own config does not change the result (`bench.sh latency` makes its own).
- Benchmarks drive a real display and inject input. On your live desktop that disturbs your session and a window
  manager may take the focus away mid-run; a nested X server (`Xephyr :5`, `DISPLAY=:5`) avoids that, but software GL
  there makes absolute numbers incomparable with a real display. The figures below come from a real X11 display.
- Only compare numbers measured together: run the terminals back to back, alternating them within a set, and pool
  several sets. Run-to-run noise on `cat` is about 1-2 %, and a cold first run can be a few percent slower.

## Baseline: montty 0.1.0

Machine: 12th Gen Intel i7-12700 (20 threads), Debian 13, kernel 6.12, X11 with Cinnamon (Muffin), nothing else running.
Compared terminals: Alacritty 0.17.0 (release build, 80x24, DejaVu Sans Mono 11) and tilix 1.9.6 (VTE 0.80, GTK 3.24,
`--geometry=80x24`, its own profile). RSS is in MB = 1000 kB.

| Metric | montty | Alacritty | tilix | Result |
|---|---|---|---|---|
| `cat` 100 MB, median of 9 runs | 0.863 s | 0.911 s | 1.325 s | 0.95x Alacritty: pass |
| Key-to-screen latency, median of 3 runs of 100 samples | 3.0 ms (p95 3.1-4.3) | 2.7-7.0 ms (p95 3.7-8.6) | 5.8 ms (p95 7.0-7.7) | within one frame: pass |
| Latency, `yes` in a neighbour pane | 13.2 ms (p95 19-21, max 24) | n/a | n/a | +10.2 ms over idle: pass (p95 exceeds a frame) |
| Idle CPU, 10 s | 0.00 % (0 ticks) | 0.00-0.10 % | 0.10 % | pass |
| Idle RSS, 1 pane | 87.7 MB | 93.9 MB | 70.6 MB | reported |
| Idle RSS, 10 panes | 98.6 MB | n/a | n/a | pass, narrowly (about 1.4 % headroom) |
| Stripped binary | 6.57 MB | n/a | n/a | pass |

Notes:
- Alacritty's latency varied between runs on the same machine (2.7 ms in one session, 7.0 ms in another); montty's was
  stable at about 3 ms in both. The gate holds either way.
- tilix and Alacritty latency and tilix `cat` were measured in a separate session from the montty figures above, with
  the same method; the `cat` ratios were stable across sessions.
- With a flooding neighbour the median latency rises to about 13 ms, close to one frame at 60 Hz: the UI thread draws
  at most one frame per refresh while the flood keeps producing output.
- The 10-pane RSS is close to its target. Treat any per-pane memory growth (grid, atlas, font sets) with care.

## Lasting findings

- `cat` throughput is limited by the PTY reader thread (read, parse, grid update), not by drawing: a flood into a
  hidden pane takes about as long as into a visible one. Wins come from the byte path, not the renderer.
- Idle CPU was once 1.5 % because winit selects raw XInput2 device events on the X11 root window, so every mouse
  motion anywhere on screen woke the main thread. montty opts out and clears that selection
  (`src/device_events.rs`; `tests/device_events.rs` covers it). Measure idle with pointer motion outside the window
  as well, since the cost depends on the user's mouse activity.
- Small `cat` differences of 1-4 % between neighbouring commits can come from code layout under fat LTO and
  `codegen-units = 1` rather than added work; confirm with pooled alternating runs before chasing them.
- The glyph atlas grows only when it fills twice within a frame and otherwise clears, so unused glyphs of evicted
  fonts (every zoom step loads a new font set) cannot push it to its maximum size; unused font sets are dropped at
  zoom, profile change, config reload, scale change and pane close.
