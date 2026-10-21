#!/bin/sh
# End-to-end check: glass-launcher against a real `glassd --mock` (glass-services).
# Usage: tools/e2e_glassd.sh GLASSD LAUNCHER
# Drives glassd's mock from its standard input and checks both logs: the launcher connects,
# moves, asks for sleep on the home card, and glassd turns the screen off and on again.
set -eu
glassd=$1
launcher=$2
# Unix socket paths are limited to about 100 bytes: keep it short.
sock=$(mktemp -u /tmp/glassd-e2e.XXXXXX)
work=$(mktemp -d)
trap 'rm -rf "$work" "$sock"' EXIT
{
  sleep 2
  echo "swipe forward"; sleep 0.5
  echo "swipe forward"; sleep 0.5
  echo "swipe down"; sleep 0.5   # back to the home card
  echo "swipe down"; sleep 0.5   # on home: the launcher asks for sleep
  echo "tap"; sleep 0.5          # wakes the screen (consumed by glassd)
  echo "scroll 0.3"; sleep 1
} | "$glassd" --mock --socket "$sock" -v >"$work/glassd.log" 2>&1 &
sleep 0.5
status=0
"$launcher" --glassd "$sock" --out "$work/frames" --renderer software --time 10:42 \
  >"$work/launcher.log" 2>&1 &
pid=$!
sleep 6
kill "$pid" 2>/dev/null || true
wait "$pid" 2>/dev/null || true
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
frames=$(ls "$work/frames" | wc -l | tr -d ' ')
echo "frames presented: $frames"
[ "$frames" -ge 4 ] || { echo "MISSING: frames"; status=1; }
exit "$status"
