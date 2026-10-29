#!/usr/bin/env bash
# Run montty inside a nested X server (Xephyr), so GUI checks never touch the real desktop or ~/.config/montty.
#
# Usage, from the repo root:
#   scripts/gui-test/session.sh start [montty args...]   start Xephyr (if not running) and montty on an empty config
#   scripts/gui-test/session.sh restart [montty args...] restart montty only, with a fresh empty config
#   scripts/gui-test/session.sh shot NAME                screenshot the nested display to $STATE/NAME.png
#   scripts/gui-test/session.sh stop                     stop montty and Xephyr
#
# Environment: GUI_TEST_DISPLAY (default :5), GUI_TEST_SIZE (1280x800), GUI_TEST_DIR (state, default
# ${TMPDIR:-/tmp}/montty-gui-test), MONTTY_BIN (target/release/montty).
# Needs Xephyr; screenshots need xwd and ffmpeg. Processes are stopped by their recorded PIDs, never by name.
set -euo pipefail

DISPLAY_NUM=${GUI_TEST_DISPLAY:-:5}
SIZE=${GUI_TEST_SIZE:-1280x800}
STATE=${GUI_TEST_DIR:-${TMPDIR:-/tmp}/montty-gui-test}
BIN=${MONTTY_BIN:-target/release/montty}

mkdir -p "$STATE"

alive() { [[ -f $1 ]] && kill -0 "$(cat "$1")" 2>/dev/null; }

stop_pid() {
    if alive "$1"; then
        kill "$(cat "$1")"
    fi
    rm -f "$1"
}

start_xephyr() {
    alive "$STATE/xephyr.pid" && return
    Xephyr "$DISPLAY_NUM" -screen "$SIZE" -ac >"$STATE/xephyr.log" 2>&1 &
    echo $! >"$STATE/xephyr.pid"
    local sock=/tmp/.X11-unix/X${DISPLAY_NUM#:}
    for _ in $(seq 50); do
        [[ -S $sock ]] && return
        sleep 0.1
    done
    echo "Xephyr did not start; see $STATE/xephyr.log" >&2
    exit 1
}

start_montty() {
    stop_pid "$STATE/montty.pid"
    # A fresh, empty config every time: montty writes its defaults there, never into the real one.
    rm -rf "$STATE/config"
    mkdir -p "$STATE/config"
    DISPLAY=$DISPLAY_NUM XDG_CONFIG_HOME=$STATE/config setsid "$BIN" "$@" >"$STATE/montty.log" 2>&1 &
    echo $! >"$STATE/montty.pid"
    sleep 2
    echo "montty pid $(cat "$STATE/montty.pid") on $DISPLAY_NUM; config $STATE/config; log $STATE/montty.log"
}

cmd=${1:-}
shift || true
case $cmd in
start)
    start_xephyr
    start_montty "$@"
    ;;
restart)
    start_montty "$@"
    ;;
shot)
    name=${1:?usage: session.sh shot NAME}
    xwd -root -display "$DISPLAY_NUM" -silent -out "$STATE/$name.xwd"
    ffmpeg -loglevel error -y -i "$STATE/$name.xwd" "$STATE/$name.png"
    rm -f "$STATE/$name.xwd"
    echo "$STATE/$name.png"
    ;;
stop)
    stop_pid "$STATE/montty.pid"
    stop_pid "$STATE/xephyr.pid"
    ;;
*)
    sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
    ;;
esac
