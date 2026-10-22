#!/bin/sh
# End-to-end check: glass-launcher against a real `glassd --mock` (glass-services).
# Usage: tools/e2e_glassd.sh GLASSD LAUNCHER
# Drives glassd's mock from its standard input and checks both logs: the launcher connects,
# moves, asks for sleep on the home card, glassd turns the screen off and on again, and a
# final scroll still draws.
set -eu
glassd=$1
launcher=$2
# Unix socket paths are limited to about 100 bytes: keep it short.
sock=$(mktemp -u /tmp/glassd-e2e.XXXXXX)
work=$(mktemp -d)
glassd_pid=
launcher_pid=
cleanup() {
  for p in $launcher_pid $glassd_pid; do
    kill "$p" 2>/dev/null || true
    wait "$p" 2>/dev/null || true
  done
  rm -rf "$work" "$sock"
}
trap cleanup EXIT

# Waits up to 10 s for a command to succeed.
wait_for() {
  i=0
  until "$@"; do
    i=$((i + 1))
    [ "$i" -lt 100 ] || { echo "timed out waiting for: $*"; return 1; }
    sleep 0.1
  done
}
frames() { ls "$work/frames" 2>/dev/null | wc -l | tr -d ' '; }
connected() { grep -q "connected to glassd" "$work/launcher.log" 2>/dev/null; }
logged() { grep -q "$2" "$1" 2>/dev/null; }

mkfifo "$work/cmds"
"$glassd" --mock --socket "$sock" -v <"$work/cmds" >"$work/glassd.log" 2>&1 &
glassd_pid=$!
exec 3>"$work/cmds" # keeps glassd's standard input open
wait_for test -S "$sock"
"$launcher" --glassd "$sock" --out "$work/frames" --renderer software --time 10:42 \
  >"$work/launcher.log" 2>&1 &
launcher_pid=$!
wait_for connected
sleep 0.3 # the subscription snapshots and the focus
echo "swipe forward" >&3
echo "swipe forward" >&3
echo "swipe down" >&3 # back to the home card
wait_for logged "$work/launcher.log" "back: to the home card"
sleep 0.6 # the animation home
echo "swipe down" >&3 # on home: the launcher asks for sleep
wait_for logged "$work/glassd.log" "screen Off (request)"
echo "tap" >&3 # wakes the screen (consumed by glassd)
wait_for logged "$work/glassd.log" "screen On (tap)"
sleep 0.3
before=$(frames)
echo "scroll 0.3" >&3
scrolled() { [ "$(frames)" -gt "$before" ]; }
wait_for scrolled || true
after=$(frames)
exec 3>&-

status=0
echo "--- glassd"; cat "$work/glassd.log"
echo "--- glass-launcher"; cat "$work/launcher.log"
check() {
  if grep -q "$2" "$1"; then echo "ok: $3"; else echo "MISSING: $3"; status=1; fi
}
check "$work/launcher.log" "connected to glassd" "the launcher connected"
check "$work/launcher.log" "back: to the home card" "a swipe down went back home"
check "$work/launcher.log" "ScreenRequest{ACTION_SLEEP}" "swipe down on home asked for sleep"
check "$work/glassd.log" "screen Off (request)" "glassd turned the screen off"
check "$work/glassd.log" "screen On (tap)" "a tap woke it"
echo "frames: $before before the scroll, $after after"
if [ "$after" -gt "$before" ]; then echo "ok: the scroll drew frames"; else echo "MISSING: scroll frames"; status=1; fi
exit "$status"
