#!/usr/bin/env bash
# montty benchmark procedure (spec section 5). See docs/bench.md for results.
#
#   bench.sh gen
#   bench.sh throughput <terminal-cmd...>
#   bench.sh idle       <terminal-cmd...>
#   bench.sh flood      <terminal-cmd...>
#   bench.sh rss10      <montty>
#   bench.sh latency    [--flood] [--samples N] <terminal-cmd...>
#
# <terminal-cmd> is the terminal executable (optionally with extra flags), e.g.
#   target/release/montty | alacritty | tilix | gnome-terminal
# The terminal's "run this command" flag is added per terminal, chosen by the
# basename of the first word (override with BENCH_STYLE=exec|tilix|gnome):
#   exec   : <term> -e cmd args...          (montty, alacritty, xterm, ...)
#   tilix  : <term> --new-process -e "cmd string"
#   gnome  : <term> --wait -- cmd args...   (idle/flood not supported: the
#            measured pid would be a launcher, not the server)
# Every run executes a command that exits on its own; BENCH_TIMEOUT (default
# 300 s) is a hard kill safety net.
set -euo pipefail

BENCH_DIR=${BENCH_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/montty-bench}
FILE=$BENCH_DIR/100mb.txt
RUNS=${BENCH_RUNS:-3}
BENCH_TIMEOUT=${BENCH_TIMEOUT:-300}

die() { echo "bench: $*" >&2; exit 1; }

style_of() {
  if [ -n "${BENCH_STYLE:-}" ]; then echo "$BENCH_STYLE"; return; fi
  case $(basename "$1") in
    tilix) echo tilix ;;
    gnome-terminal) echo gnome ;;
    *) echo exec ;;
  esac
}

# run_in_term <terminal words...> -- <cmd words...> : runs in background, sets TERM_PID
spawn() {
  local -a term=() cmd=()
  while [ "$1" != "--" ]; do term+=("$1"); shift; done
  shift
  cmd=("$@")
  case $(style_of "${term[0]}") in
    exec)  "${term[@]}" -e "${cmd[@]}" >/dev/null 2>&1 & ;;
    tilix) "${term[@]}" --new-process -e "$(printf '%q ' "${cmd[@]}")" >/dev/null 2>&1 & ;;
    gnome) "${term[@]}" --wait -- "${cmd[@]}" >/dev/null 2>&1 & ;;
  esac
  TERM_PID=$!
}

# stop_term pid : SIGTERM, then SIGKILL after 3 s
stop_term() {
  local pid=$1 i
  kill "$pid" 2>/dev/null || return 0
  for i in $(seq 15); do kill -0 "$pid" 2>/dev/null || return 0; sleep 0.2; done
  kill -9 "$pid" 2>/dev/null || true
}
TERM_PID=
cfg=   # rss10's temporary config dir
cleanup() {
  [ -z "$TERM_PID" ] || stop_term "$TERM_PID"
  [ -z "$cfg" ] || rm -rf "$cfg"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# wait_for_exit pid : waits up to BENCH_TIMEOUT s; sets WAIT_STATUS to the exit
# status, or KILLED if it had to be stopped. Returns 0 only for status 0.
WAIT_STATUS=
wait_for_exit() {
  local pid=$1 t=0
  while kill -0 "$pid" 2>/dev/null; do
    sleep 0.2; t=$((t + 1))
    if [ $((t / 5)) -ge "$BENCH_TIMEOUT" ]; then
      stop_term "$pid"; wait "$pid" 2>/dev/null || true
      WAIT_STATUS=KILLED; echo "bench: timeout, killed $pid" >&2; return 1
    fi
  done
  WAIT_STATUS=0; wait "$pid" 2>/dev/null || WAIT_STATUS=$?
  [ "$WAIT_STATUS" = 0 ]
}

ticks() { awk '{print $14 + $15}' "/proc/$1/stat"; }   # utime+stime
median() { sort -g | awk '{a[NR]=$1} END{print (NR%2) ? a[(NR+1)/2] : (a[NR/2]+a[NR/2+1])/2}'; }

cmd_gen() {
  mkdir -p "$BENCH_DIR"
  # ~1 MiB deterministic block of mixed plain and ANSI-coloured lines, repeated to 100 MiB.
  local blk=$BENCH_DIR/block.txt
  awk 'BEGIN{
    for (i = 0; i < 12000; i++) {
      n = i % 8
      if (n == 0)      printf "\033[3%dmline %d: the quick brown fox jumps over the lazy dog\033[0m\n", i % 7 + 1, i
      else if (n == 1) printf "%06d  0123456789 abcdefghijklmnopqrstuvwxyz ABCDEFGHIJKLMNOPQRSTUVWXYZ {}[]()<>;:,.\n", i
      else if (n == 2) printf "\033[1;3%d;4%dmbold %d\033[0m plain text continues here with some more words to fill the line %d\n", i % 7 + 1, (i + 3) % 7, i, i * 7
      else             printf "%d lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor %d\n", i, i * 31 % 1009
    }}' > "$blk"
  local bs; bs=$(stat -c %s "$blk")
  local n=$(( (100 * 1024 * 1024) / bs + 1 ))
  : > "$FILE"
  for _ in $(seq "$n"); do cat "$blk" >> "$FILE"; done
  truncate -s $((100 * 1024 * 1024)) "$FILE"
  rm -f "$blk"
  ls -l "$FILE"; sha256sum "$FILE"
}

cmd_throughput() {
  [ $# -ge 1 ] || die "usage: throughput <terminal-cmd...>"
  [ -f "$FILE" ] || die "run '$0 gen' first"
  local res=$BENCH_DIR/result.$$ out=() i
  for i in $(seq "$RUNS"); do
    rm -f "$res"
    spawn "$@" -- sh -c 's=$(date +%s.%N); cat "$1"; e=$(date +%s.%N); echo "$s $e" > "$2"' sh "$FILE" "$res"
    wait_for_exit "$TERM_PID" || true
    [ -s "$res" ] || die "no result written (terminal did not run the command?)"
    local d; d=$(awk '{printf "%.3f", $2 - $1}' "$res")
    echo "run $i: ${d}s" >&2
    out+=("$d"); sleep 1   # let the terminal (gnome-terminal server) settle
  done
  rm -f "$res"
  echo "median: $(printf '%s\n' "${out[@]}" | median) s  (runs: ${out[*]})"
}

cmd_idle() {
  [ $# -ge 1 ] || die "usage: idle <terminal-cmd...>"
  [ "$(style_of "$1")" != gnome ] || die "idle: gnome-terminal unsupported (server pid)"
  local hz; hz=$(getconf CLK_TCK)
  spawn "$@" -- sleep 40
  local pid=$TERM_PID
  sleep 3
  kill -0 "$pid" 2>/dev/null || die "terminal exited early"
  local t0 t1; t0=$(ticks "$pid"); sleep 10; t1=$(ticks "$pid")
  awk -v a="$t0" -v b="$t1" -v hz="$hz" 'BEGIN{printf "idle CPU: %.2f %% (over 10 s, %d ticks)\n", (b - a) / hz / 10 * 100, b - a}'
  grep -E 'VmRSS|VmHWM' "/proc/$pid/status"
  stop_term "$pid"; wait "$pid" 2>/dev/null || true
}

# flood: run `yes` for 5 s inside the terminal; report the terminal's CPU and exit cleanliness.
cmd_flood() {
  [ $# -ge 1 ] || die "usage: flood <terminal-cmd...>"
  [ "$(style_of "$1")" != gnome ] || die "flood: gnome-terminal unsupported (server pid)"
  local hz; hz=$(getconf CLK_TCK)
  spawn "$@" -- timeout 5 yes
  local pid=$TERM_PID
  sleep 1
  kill -0 "$pid" 2>/dev/null || die "terminal exited early"
  local t0 t1; t0=$(ticks "$pid"); sleep 3; t1=$(ticks "$pid")
  awk -v a="$t0" -v b="$t1" -v hz="$hz" 'BEGIN{printf "flood CPU: %.1f %% of one core (3 s window)\n", (b - a) / hz / 3 * 100}'
  grep VmRSS "/proc/$pid/status"
  if wait_for_exit "$pid"; then echo "terminal exited cleanly (status 0) after timeout ended"
  else echo "terminal did NOT exit cleanly: status $WAIT_STATUS"; return 1; fi
}

# rss10: open a 10-pane session (needs DISPLAY), idle 5 s, print montty's RSS.
cmd_rss10() {
  [ $# -eq 1 ] || die "usage: rss10 <montty>"
  [ -n "${DISPLAY:-}" ] || die "rss10: DISPLAY is not set"
  cfg=$(mktemp -d)   # global: cleanup() runs from the EXIT trap, after this function returns
  mkdir -p "$cfg/montty/sessions"
  python3 - "$cfg/montty/sessions/ten.json" <<'PY'
import json, sys
node = {"profile": ""}
for _ in range(9):
    node = {"split": "right", "ratio": 0.5, "a": {"profile": ""}, "b": node}
json.dump({"version": 1, "active_tab": 0, "tabs": [{"focused": 0, "root": node}]}, open(sys.argv[1], "w"))
PY
  XDG_CONFIG_HOME=$cfg "$1" --session ten >/dev/null 2>&1 &
  TERM_PID=$!
  local pid=$TERM_PID i n=0
  for i in $(seq 50); do
    kill -0 "$pid" 2>/dev/null || die "rss10: montty exited early"
    # child shells only: not the image-decoder helper (`/proc/self/exe --decode-image ...`)
    n=$({ ps -o args= --ppid "$pid" | grep -vF -e '--decode-image' || true; } | wc -l)
    [ "$n" -lt 10 ] || break
    sleep 0.2
  done
  [ "$n" -ge 10 ] || die "rss10: only $n of 10 child shells appeared within 10 s"
  sleep 5
  kill -0 "$pid" 2>/dev/null || die "rss10: montty died while idle"
  awk '/VmRSS/{r=$2} /VmHWM/{h=$2} END{printf "rss10: VmRSS=%d kB VmHWM=%d kB (10 panes)\n", r, h}' "/proc/$pid/status"
}

# latency: key-to-screen latency via XTest (scripts/latency.py; needs DISPLAY).
# The terminal runs `cat`; --flood (montty only) adds a `yes` split neighbour.
# One summary line per run (3 runs of --samples samples, default 100).
cmd_latency() {
  local -a opts=()
  while [ $# -gt 0 ]; do
    case $1 in
      --flood) opts+=(--flood); shift ;;
      --samples) [ $# -ge 2 ] || die "latency: --samples needs a number"; opts+=(--samples "$2"); shift 2 ;;
      --) shift; break ;;
      *) break ;;
    esac
  done
  [ $# -ge 1 ] || die "usage: latency [--flood] [--samples N] <terminal-cmd...>"
  [ -n "${DISPLAY:-}" ] || die "latency: DISPLAY is not set"
  local style; style=$(style_of "$1")
  [ "$style" != gnome ] || die "latency: gnome-terminal unsupported (server pid)"
  python3 "$(dirname "$0")/latency.py" "${opts[@]}" --style "$style" "$@"
}

sub=${1:-}; [ $# -gt 0 ] && shift
case $sub in
  gen|throughput|idle|flood|rss10|latency) "cmd_$sub" "$@" ;;
  *) sed -n '2,13p' "$0"; exit 2 ;;
esac
