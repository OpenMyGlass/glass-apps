#!/bin/sh
# Host test of glass-launcher-supervisor.sh with fake launchers and fake GPU tools.
# Usage: native/fbwin/test_supervisor.sh
# SHELL_UNDER_TEST runs the supervisor under another shell, for example "busybox sh".
here=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d) || exit 1
# A private copy, so that pkill finds this run's supervisors only. Outside $T: sup() cleans up with
# pkill on $T, and a watcher left behind by a supervisor must survive that to be caught.
S=$(mktemp -d) || exit 1
SUP=$S/glass-launcher-supervisor.sh
cp "${SUP_UNDER_TEST:-$here/glass-launcher-supervisor.sh}" "$SUP" || exit 1
trap 'pkill -KILL -f "$S/" 2>/dev/null; pkill -KILL -f "$T/" 2>/dev/null; rm -rf "$T" "$S"' EXIT
fails=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; sed 's/^/     /' "$T/log" 2>/dev/null; fails=$((fails + 1)); }

mkdir -p "$T/fake"
# Fakes stay shell scripts while they hang, so that "$T/" or "$W/" in the command line finds them.
for t in insmod mknod; do
    printf '#!/bin/sh\necho "%s $*" >> "%s/calls"\n[ -e "%s/%s-hangs" ] && while :; do sleep 1; done\nexit 0\n' "$t" "$T" "$T" "$t" > "$T/fake/$t"
    chmod 755 "$T/fake/$t"
done
printf '#!/bin/sh\nwhile :; do sleep 1; done\n' > "$T/fake/hang"
# A real sync can take longer than the short limits here when the host is busy.
# A real sync can also be slow on purpose: $T/sync-slow.
printf '#!/bin/sh\n[ -e "%s/sync-slow" ] && sleep 0.6\nexit 0\n' "$T" > "$T/fake/sync"
# A launcher that takes half a second to die after SIGKILL, as SGX may when it lets go.
printf '#!/bin/sh\n(sleep 0.5; kill -KILL "$1") &\n' > "$T/fake/slowkill"
chmod 755 "$T/fake/hang" "$T/fake/sync" "$T/fake/slowkill"

# A fresh world, with $W/state on a persistent filesystem. $1: what the GPU launcher does.
world() {
    W=$T/w; pkill -KILL -f "$S/" 2>/dev/null; pkill -KILL -f "$T/" 2>/dev/null; rm -rf "$W"; rm -f "$T/calls" "$T/log" "$T"/*-hangs "$T/sync-slow"
    wait_forever='while :; do sleep 1; done'
    SUP_ENV=; SUP_SIGNAL=TERM; SUP_ARGS='--glassd sock'; sup_lingered=
    mkdir -p "$W/system/bin" "$W/system/vendor/bin" "$W/system/vendor/lib/hw" "$W/lib" "$W/dev" "$W/proc" "$W/state" "$W/run"
    : > "$W/system/bin/linker"; : > "$W/system/vendor/lib/hw/gralloc.omap4430.so"; : > "$W/lib/libglass_fbwin.so"; : > "$W/lib/module.ko"; : > "$W/dev/fb0"
    : > "$W/proc/modules"; echo '242 pvrsrvkm' > "$W/proc/devices"
    W_REAL=$(cd "$W" && pwd -P)
    printf 'rootfs / rootfs rw 0 0\n/dev/mmcblk0p1 %s/state ext4 rw 0 0\n' "$W_REAL" > "$W/proc/mounts"
    printf '#!/bin/sh\necho "ctl $*" >> "%s/calls"\necho "LD=$LD_LIBRARY_PATH" >> "%s/calls"\n' "$T" "$T" > "$W/system/vendor/bin/pvrsrvctl_SGX540_120"
    # With $W/soft-talks it writes to stderr, and notes a failed write the way a Rust launcher would panic.
    cat > "$W/soft" <<G
#!/bin/sh
echo "soft \$*" >> "$T/calls"
echo soft > "$W/started"
if [ -e "$W/soft-talks" ]; then
    while :; do echo tick >&2 || { echo epipe >> "$T/calls"; exit 1; }; sleep 0.1; done
fi
$wait_forever
G
    case $1 in
        beat) body='i=0; while :; do i=$((i+1)); echo $i > "$BEATF"; sleep 0.05; done' ;;
        crash) body='exit 3' ;;
        silent) body=$wait_forever ;;
        crash-after-beats) body='i=0; while [ $i -lt 30 ]; do i=$((i+1)); echo $i > "$BEATF"; sleep 0.05; done; exit 3' ;;
        flood) body="sleep 0.5; echo flooded >> \"$T/calls\"; i=0; while [ \$i -lt 2000 ]; do echo \"flood line \$i: ................................................................................\" >&2; i=\$((i+1)); done; $wait_forever" ;;
        beat-then-slowsync) body="i=0; while [ \$i -lt 30 ]; do i=\$((i+1)); echo \$i > \"\$BEATF\"; sleep 0.05; done; : > \"$T/sync-slow\"; $wait_forever" ;;
        beat-then-silent) body="i=0; while [ \$i -lt 30 ]; do i=\$((i+1)); echo \$i > \"\$BEATF\"; sleep 0.05; done; $wait_forever" ;;
        crash-and-break-counter) body="mkdir -p \"$W/state/gpu-failures.new\"; exit 3" ;;
        replaced) body="echo 99999 > \"$W/run/launcher-pid\"; $wait_forever" ;;
    esac
    cat > "$W/gles" <<G
#!/bin/sh
echo "gles \$* LD=\$LD_LIBRARY_PATH" >> "$T/calls"
while [ \$# -gt 0 ]; do [ "\$1" = --heartbeat ] && BEATF=\$2; shift; done
$body
G
    chmod 755 "$W/system/vendor/bin/pvrsrvctl_SGX540_120" "$W/soft" "$W/gles"
}

launch() { # in a subshell: it becomes the supervisor, and a signal to its PID reaches it
    exec env PATH="$T/fake:$PATH" GLASS_SYSTEM="$W/system" GLASS_GPU_MODULE="$W/lib/module.ko" \
        GLASS_GLES_LAUNCHER="$W/gles" GLASS_SOFT_LAUNCHER="$W/soft" GLASS_FBWIN="$W/lib/libglass_fbwin.so" \
        GLASS_FB="$W/dev/fb0" GLASS_DEV="$W/dev" GLASS_PROC="$W/proc" GLASS_LAUNCHER_STATE="$W/state" \
        GLASS_INSMOD="$T/fake/insmod" GLASS_MKNOD="$T/fake/mknod" GLASS_SYNC="$T/fake/sync" \
        GLASS_LAUNCHER_RUN="$W/run" GLASS_REBOOT="touch $W/rebooted" GLASS_SUPERVISOR_POLL_S=0.2 \
        GLASS_SUPERVISOR_KILL_POLLS=1 GLASS_SUPERVISOR_REBOOT_WAIT_S=0.2 GLASS_SUPERVISOR_BOUND_S=5 \
        $SUP_ENV ${SHELL_UNDER_TEST:-sh} "$SUP" $SUP_ARGS
}

sup() { # seconds, then $SUP_SIGNAL
    (launch) >> "$T/log" 2>&1 &
    _p=$!
    sleep "$1"
    kill -"$SUP_SIGNAL" "$_p" 2>/dev/null
    sleep "${SUP_STOP_WAIT:-0.2}"
    if kill -0 "$_p" 2>/dev/null; then
        sup_lingered=1
        kill -KILL "$_p" 2>/dev/null
    fi
    wait "$_p" 2>/dev/null
    pkill -KILL -f "$W/" 2>/dev/null
    pkill -KILL -f "$T/" 2>/dev/null
    sleep 0.1
}
# The next boot: what is in RAM is gone, the state stays.
reboot_world() { rm -rf "$W/run" "$W/rebooted" "$W/dev/__properties__"; mkdir -p "$W/run"; rm -f "$T/calls"; }
called() { grep -q -- "$1" "$T/calls" 2>/dev/null; }
count() { grep -c -- "$1" "$T/calls" 2>/dev/null; }
counter() { cat "$W/state/$1" 2>/dev/null; }

world beat; rm "$W/system/vendor/bin/pvrsrvctl_SGX540_120"; sup 1
if called '^soft' && ! called '^gles' && ! called '^insmod' && grep -q 'software renderer' "$T/log"; then ok "no blobs: software, nothing loaded"; else bad "no blobs"; fi

world beat; : > "$W/state/no-gpu"; sup 1
if called '^soft' && ! called '^gles' && grep -q 'no-gpu is set' "$T/log"; then ok "marker: software"; else bad "marker"; fi

world beat; sup 2.5
if called '^gles --display fbwin .*libglass_fbwin.so --heartbeat .*--glassd sock' && ! called '^soft' && called "LD=$W/system/lib:" \
    && called '^ctl --start --no-module' && called '^insmod' && called '^mknod -m 666 .*pvrsrvkm c 242 0' \
    && [ "$(counter gpu-failures)" = 0 ] && grep -q healthy "$T/log" && [ -L "$W/dev/graphics/fb0" ]; then
    ok "healthy GPU launcher: counter back to 0"; else bad "healthy"; fi
python3 - "$W/dev/__properties__" <<'P' && ok "property area bytes" || bad "property area bytes"
import struct,sys
d=open(sys.argv[1],'rb').read()
e=bytearray(128*1024); struct.pack_into('<4I',e,0,32,0,0x504f5250,0xfc6ed0ab)
sys.exit(0 if d==bytes(e) else 1)
P

world crash; sup 2
if [ "$(count '^gles')" = 2 ] && called '^soft' && [ -e "$W/state/no-gpu" ]; then ok "two crashes: marker, software"; else bad "two crashes ($(count '^gles'))"; fi

world silent; sup 2.5
if [ "$(count '^gles')" = 1 ] && called '^soft' && ! [ -e "$W/state/no-gpu" ] && [ "$(counter gpu-failures)" = 1 ] && ! [ -e "$W/rebooted" ]; then
    ok "hang: killed, software, one failure"; else bad "hang"; fi

world silent; echo 1 > "$W/state/gpu-failures"; sup 2.5
if [ "$(counter gpu-failures)" = 2 ] && called '^soft'; then ok "second bad boot: two failures"; else bad "second bad boot"; fi
reboot_world; sup 1
if [ -e "$W/state/no-gpu" ] && ! called '^gles' && called '^soft'; then ok "third boot: marker, GPU not tried"; else bad "third boot"; fi

world crash; rm -f "$W/run/gpu-up"; echo 'junk' > "$W/state/gpu-failures"; sup 1
if [ "$(count '^gles')" -ge 1 ]; then ok "unreadable counter counts as 0"; else bad "junk counter"; fi

world beat; printf '#!/bin/sh\necho no >&2; exit 1\n' > "$W/system/vendor/bin/pvrsrvctl_SGX540_120"; sup 1
if called '^soft' && ! called '^gles' && grep -q 'pvrsrvctl failed: no' "$T/log"; then ok "pvrsrvctl fails: software"; else bad "pvrsrvctl fails"; fi

world beat; echo 'pvrsrvkm 1 0' > "$W/proc/modules"; sup 1
if ! called '^insmod' && called '^gles'; then ok "module already loaded"; else bad "module loaded"; fi

world crash; printf '#!/bin/sh\nexit 2\n' > "$W/soft"; sup 4.5
if [ "$(grep -c 'starting the software launcher' "$T/log")" -ge 2 ]; then ok "software launcher respawns"; else bad "respawn"; fi

# Bring-up that never ends. GLASS_KILL=: stands for a process in D state: the kill does nothing.
world beat; : > "$T/insmod-hangs"; SUP_ENV=GLASS_SUPERVISOR_BOUND_S=0.5; sup 1.5
if called '^soft' && ! called '^gles' && [ "$(counter gpu-failures)" = 1 ]; then ok "insmod hangs: software, one failure"; else bad "insmod hangs"; fi

world beat; : > "$T/insmod-hangs"; SUP_ENV="GLASS_KILL=: GLASS_SUPERVISOR_BOUND_S=0.5"; sup 1.5
if called '^soft' && ! called '^gles'; then ok "insmod cannot be killed: software"; else bad "insmod cannot be killed"; fi

stuck_ctl='#!/bin/sh
echo "ctl $*" >> "'"$T"'/calls"
while :; do sleep 1; done'
world beat; printf '%s\n' "$stuck_ctl" > "$W/system/vendor/bin/pvrsrvctl_SGX540_120"; SUP_ENV="GLASS_KILL=: GLASS_SUPERVISOR_BOUND_S=0.5"; sup 1.5
if called '^soft' && ! called '^gles' && [ "$(counter gpu-failures)" = 1 ]; then ok "pvrsrvctl cannot be killed: software"; else bad "pvrsrvctl cannot be killed"; fi

# The kill works, but a child still holds the output: a $(...) would wait for its EOF.
world beat; printf '#!/bin/sh\n(while :; do sleep 1; done) &\nwhile :; do sleep 1; done\n' > "$W/system/vendor/bin/pvrsrvctl_SGX540_120"; SUP_ENV=GLASS_SUPERVISOR_BOUND_S=0.5; sup 1.5
if called '^soft' && ! called '^gles'; then ok "pvrsrvctl's child holds its output: software"; else bad "pvrsrvctl's child holds its output"; fi

world beat; : > "$T/insmod-hangs"; SUP_ENV=GLASS_SUPERVISOR_BOUND_S=0.5; sup 1.2; reboot_world; sup 1.2; reboot_world; sup 1
if [ -e "$W/state/no-gpu" ] && ! called '^insmod' && called '^soft'; then ok "two hung bring-ups: marker"; else bad "two hung bring-ups"; fi

# State that cannot be written. A directory in the way stops root as well.
world beat; mkdir "$W/state/gpu-failures.new"; sup 1.5
if called '^soft' && ! called '^gles' && ! called '^insmod' && grep -q 'cannot write' "$T/log"; then ok "counter cannot be written: GPU not tried"; else bad "counter cannot be written"; fi

world crash-and-break-counter; sup 2.5
if [ "$(count '^gles')" = 1 ] && called '^soft'; then ok "counter breaks after a crash: no restart on the GPU"; else bad "counter breaks ($(count '^gles'))"; fi

# Without the marker's file being writable the supervisor must still reach software.
world beat; echo 2 > "$W/state/gpu-failures"; ln -s /nonexistent/dir/x "$W/state/no-gpu"; sup 1
if called '^soft' && ! called '^gles'; then ok "marker cannot be written: software"; else bad "marker cannot be written"; fi

# A read that blocks stands for a stuck disk.
world beat; mkfifo "$W/state/gpu-failures"; SUP_ENV=GLASS_SUPERVISOR_BOUND_S=0.5; sup 1.5
if called '^soft' && ! called '^gles'; then ok "state read blocks: software"; else bad "state read blocks"; fi

# Is the state persistent?
world beat; echo 'rootfs / rootfs rw 0 0' > "$W/proc/mounts"; sup 1
if called '^soft' && ! called '^gles' && ! [ -e "$W/state/gpu-failures" ] && grep -q 'does not survive a reboot' "$T/log"; then ok "state on rootfs: GPU refused"; else bad "state on rootfs"; fi

world beat; rm "$W/proc/mounts"; sup 1
if called '^soft' && ! called '^gles'; then ok "mounts unreadable: GPU refused"; else bad "mounts unreadable"; fi

world beat; echo "tmpfs $W_REAL/state tmpfs rw 0 0" >> "$W/proc/mounts"; sup 1
if called '^soft' && ! called '^gles'; then ok "tmpfs mounted over the state: GPU refused"; else bad "tmpfs over the state"; fi

world beat; echo "tmpfs ${W_REAL}2 tmpfs rw 0 0" >> "$W/proc/mounts"; sup 1
if called '^gles' && ! called '^soft'; then ok "a mount with a similar path does not count"; else bad "similar path"; fi

world beat; printf 'rootfs / rootfs rw 0 0\n/dev/root / ext4 rw 0 0\n' > "$W/proc/mounts"; sup 1
if called '^gles' && ! called '^soft'; then ok "state on a real root filesystem: GPU tried"; else bad "real root"; fi

world beat; echo 'rootfs / rootfs rw 0 0' > "$W/proc/mounts"; SUP_ENV=GLASS_GPU_ALLOW_VOLATILE_STATE=1; sup 1
if called '^gles' && ! called '^soft' && grep -q 'WARNING.*boot loop' "$T/log"; then ok "volatile state with the override: GPU tried, warned"; else bad "override"; fi

# A launcher that runs well, then cannot be killed.
world beat-then-silent; SUP_ENV='GLASS_KILL=:'; sup 3.5
if [ -e "$W/rebooted" ] && [ "$(counter gpu-wedges)" = 1 ] && [ "$(counter gpu-failures)" = 0 ] && [ "$(count '^gles')" = 1 ]; then ok "stuck after running well: counted, rebooted"; else bad "stuck after running well"; fi
if called '^soft' && grep -q 'no reboot after' "$T/log"; then ok "reboot returns: software launcher"; else bad "reboot returns"; fi
reboot_world; sup 3.5
if [ "$(counter gpu-wedges)" = 2 ] && [ -e "$W/rebooted" ]; then ok "stuck again: two counted"; else bad "stuck again ($(counter gpu-wedges))"; fi
reboot_world; sup 1
if [ -e "$W/state/no-gpu" ] && ! called '^gles' && called '^soft' && ! [ -e "$W/rebooted" ]; then ok "third boot after two stuck drivers: marker, no reboot"; else bad "third stuck boot"; fi

world silent; SUP_ENV="GLASS_KILL=: GLASS_REBOOT=$T/fake/hang"; sup 2
if called '^soft'; then ok "reboot never returns: software launcher"; else bad "reboot never returns"; fi

world beat; echo 1 > "$W/state/gpu-wedges"; SUP_ENV=GLASS_SUPERVISOR_WEDGE_CLEAR_POLLS=8; sup 2.5
if ! [ -e "$W/state/gpu-wedges" ] && called '^gles'; then ok "long healthy run forgets the stuck drivers"; else bad "wedges forgotten"; fi

world beat; echo 1 > "$W/state/gpu-wedges"; sup 2
if [ "$(counter gpu-wedges)" = 1 ]; then ok "a short healthy run does not forget them"; else bad "wedges kept"; fi

# A launcher that dies late after the kill is not stuck.
world beat-then-silent; SUP_ENV="GLASS_KILL=$T/fake/slowkill GLASS_SUPERVISOR_KILL_POLLS=4"; sup 4
if ! [ -e "$W/rebooted" ] && ! [ -e "$W/state/gpu-wedges" ] && called '^soft' && [ "$(count '^gles')" = 1 ]; then ok "launcher dies 0.5 s after the kill: no reboot, software"; else bad "slow death"; fi

world beat-then-slowsync; SUP_ENV="GLASS_KILL=$T/fake/slowkill"; sup 5
if ! [ -e "$W/rebooted" ] && ! [ -e "$W/state/gpu-wedges" ] && called '^soft'; then ok "launcher dies while the reboot is prepared: no reboot, not counted"; else bad "death before the reboot ($(counter gpu-wedges))"; fi

# The watcher never outlives its launcher.
world silent; sup 0.3; sleep 1.5
if ! [ -e "$W/run/hung" ] && ! [ -e "$W/rebooted" ] && [ -z "$sup_lingered" ]; then ok "SIGTERM: the watcher goes too"; else bad "SIGTERM and the watcher"; fi

world silent; SUP_SIGNAL=KILL; sup 0.3; sleep 1.5
if ! [ -e "$W/run/hung" ] && ! [ -e "$W/rebooted" ]; then ok "supervisor killed: an orphan watcher does nothing"; else bad "orphan watcher"; fi

world replaced; sup 2
if ! [ -e "$W/run/hung" ] && ! called '^soft' && ! grep -q 'killed$' "$T/log"; then ok "launcher replaced: the old watcher does nothing"; else bad "old watcher"; fi

# Clean stops and power cuts.
world beat; sup 0.3; reboot_world; sup 0.3; reboot_world; sup 0.3
if [ "$(counter gpu-failures)" = 0 ] && ! [ -e "$W/state/no-gpu" ] && called '^gles' && [ -z "$sup_lingered" ]; then ok "clean stops before 20 s: nothing counted"; else bad "clean stops ($(counter gpu-failures))"; fi

world beat; echo 1 > "$W/state/gpu-failures"; sup 0.3
if [ "$(counter gpu-failures)" = 1 ]; then ok "clean stop: an earlier failure stays"; else bad "earlier failure ($(counter gpu-failures))"; fi

# The stop waits for the count to be written, then puts it back.
world beat; : > "$T/sync-slow"; SUP_STOP_WAIT=2.5; sup 0.3; SUP_STOP_WAIT=
if { [ "$(counter gpu-failures)" = 0 ] || ! [ -e "$W/state/gpu-failures" ]; } && [ -z "$sup_lingered" ] && ! [ -e "$W/state/gpu-failures.new" ]; then ok "clean stop while the attempt is being counted: taken back"; else bad "stop during the count ($(counter gpu-failures))"; fi

world beat; SUP_SIGNAL=KILL; sup 0.3
if [ "$(counter gpu-failures)" = 1 ]; then ok "power cut: counted"; else bad "power cut ($(counter gpu-failures))"; fi

world crash; sup 2
if [ "$(counter gpu-failures)" = 2 ]; then ok "clean stop after failures: they stay"; else bad "stop after failures"; fi

world beat; : > "$W/state/no-gpu"; echo 2 > "$W/state/gpu-failures"; echo 2 > "$W/state/gpu-wedges"; SUP_ARGS=--clear-gpu-state; (launch) >> "$T/log" 2>&1; rc=$?
if [ "$rc" = 0 ] && ! [ -e "$W/state/no-gpu" ] && ! [ -e "$W/state/gpu-failures" ] && ! [ -e "$W/state/gpu-wedges" ] && ! called '^soft' && ! called '^gles'; then
    ok "--clear-gpu-state: clears, starts nothing"; else bad "clear ($rc)"; fi
SUP_ARGS='--glassd sock'; sup 1
if called '^gles'; then ok "after --clear-gpu-state: GPU tried"; else bad "after clear"; fi

# A bound on restarts in one boot.
world crash-after-beats; sup 16
if [ "$(count '^gles')" = 5 ] && called '^soft' && ! [ -e "$W/state/no-gpu" ] && [ "$(counter gpu-failures)" = 0 ]; then
    ok "crashes after running well: 5 starts, software, no marker"; else bad "crash after beats ($(count '^gles'))"; fi

# The reader of the log is gone.
world crash; ( (launch) 2>&1; echo "supervisor ended" > "$T/ended") | true &
sleep 2.5
if called '^soft' && ! [ -e "$T/ended" ]; then ok "log reader gone: the supervisor goes on"; else bad "log reader gone"; fi
pkill -KILL -f "$S/" 2>/dev/null
wait 2>/dev/null

# Only the relay is killed: its keeper starts it again and the launcher's lines go on.
world beat; rm "$W/system/vendor/bin/pvrsrvctl_SGX540_120"; : > "$W/soft-talks"
(launch) >> "$T/log" 2>&1 &
sup_p=$!
sleep 0.5
kill -KILL "$(cat "$W/run/relay.pid")" 2>/dev/null
sleep 0.1
before=$(grep -c tick "$T/log")
sleep 2
after=$(grep -c tick "$T/log")
if [ "$after" -gt $((before + 5)) ] && [ "$(count '^soft')" = 1 ]; then ok "relay killed: started again, the launcher's lines go on"; else bad "relay restart ($before $after)"; fi
kill "$sup_p" 2>/dev/null; sleep 0.2; kill -KILL "$sup_p" 2>/dev/null; wait "$sup_p" 2>/dev/null
pkill -KILL -f "$S/" 2>/dev/null; pkill -KILL -f "$T/" 2>/dev/null

# The relay is killed and the GPU launcher fills the FIFO, then hangs: it must still be killed,
# and the software launcher must start.
world flood; SUP_ENV=GLASS_SUPERVISOR_KILL_POLLS=1
(launch) >> "$T/log" 2>&1 &
sup_p=$!
sleep 0.3
kill -KILL "$(cat "$W/run/relay-keeper.pid")" "$(cat "$W/run/relay.pid")" 2>/dev/null
sleep 3
if called '^soft' && [ -e "$W/run/hung" ] && [ "$(count '^gles')" = 1 ] && called flooded && grep -q 'starting the software launcher' "$T/log" \
    && grep -q 'flood line 600:' "$T/log"; then
    ok "relay killed, FIFO full: launcher killed, software, relay back"; else bad "relay killed"; fi
kill "$sup_p" 2>/dev/null; sleep 0.2; kill -KILL "$sup_p" 2>/dev/null; wait "$sup_p" 2>/dev/null
pkill -KILL -f "$S/" 2>/dev/null; pkill -KILL -f "$T/" 2>/dev/null

world beat; rm "$W/system/vendor/bin/pvrsrvctl_SGX540_120"; : > "$W/soft-talks"; : > "$W/kmsg"; SUP_ENV="GLASS_LOG_FALLBACK=$W/kmsg"
( (launch) 2>&1 ) | { IFS= read -r _; } &
sleep 2
if [ "$(count '^soft')" = 1 ] && ! called epipe && grep -q tick "$W/kmsg" && grep -q 'log reader is gone' "$W/kmsg"; then
    ok "log reader gone: the launcher's writes go on, into the fallback"; else bad "launcher after the log reader ($(count '^soft'))"; fi
pkill -KILL -f "$S/" 2>/dev/null
pkill -KILL -f "$T/" 2>/dev/null
wait 2>/dev/null

[ "$fails" = 0 ] && echo "supervisor tests passed" || { echo "$fails failed"; exit 1; }
