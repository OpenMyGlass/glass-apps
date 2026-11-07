#!/bin/sh
# Model B: starts the launcher on the GPU when that can work, on the software renderer when it
# cannot, and never leaves the Glass without a UI. Logs to stderr; the init sends it to kmsg.
#
# Usage: glass-launcher-supervisor.sh [launcher arguments...]
#        glass-launcher-supervisor.sh --clear-gpu-state    forget the marker and the counters
#
# - No blobs (the fully open configuration), or a no-gpu marker: software at once.
# - $STATE must survive a reboot, or a GPU that panics the kernel is a boot loop: nothing could
#   count the crashes. The init mounts it before starting this script. On rootfs, ramfs, tmpfs or
#   anything not known to persist the GPU is refused, unless GLASS_GPU_ALLOW_VOLATILE_STATE=1, for
#   test images booted from RAM only. If the counters cannot be written the GPU is refused too.
# - Every GPU attempt counts as a failure until the launcher has beaten for 20 s, also across
#   reboots, so a kernel crash in that time counts. Two in a row write the marker. A SIGTERM takes
#   the count back: the init sends it on a clean stop, before unmounting $STATE. A power cut cannot.
# - State checks and GPU bring-up have a time limit. Past it their processes are killed if they
#   can be and left behind if not, and this boot goes on in software.
# - A launcher that stops beating for 20 s of awake time is killed and this boot goes on in
#   software. If it cannot be killed the driver is stuck and only a reboot brings the UI back: that
#   reboot is counted, two in a row write the marker, and an hour of beats clears the count. A
#   launcher that dies within 30 s of the kill, or before the reboot, is not stuck. If the reboot
#   does not happen the software launcher starts anyway.
# - The launchers write their output to a relay, never to the init's pipe: if whoever reads the
#   log dies, the relay goes on into /dev/kmsg (GLASS_LOG_FALLBACK) and no launcher sees EPIPE.
# - A launcher that crashes after a healthy start is restarted, up to 5 starts per boot.
# - The marker stays until --clear-gpu-state or a reinstall.
#
# Every path can be overridden from the environment, which is how the tests run this on a host.
#
# Do not write ": > file" here: a failed redirection on a special builtin exits the shell.

SYSTEM=${GLASS_SYSTEM:-/system}
MODULE=${GLASS_GPU_MODULE:-/lib/modules/pvrsrvkm_sgx540_120.ko}
GLES_LAUNCHER=${GLASS_GLES_LAUNCHER:-/usr/libexec/glass-launcher-gles}
SOFT_LAUNCHER=${GLASS_SOFT_LAUNCHER:-/usr/bin/glass-launcher}
FBWIN=${GLASS_FBWIN:-/usr/lib/glass/libglass_fbwin.so}
FB=${GLASS_FB:-/dev/fb0}
DEV=${GLASS_DEV:-/dev}
PROC=${GLASS_PROC:-/proc}
STATE=${GLASS_LAUNCHER_STATE:-/var/lib/glass-launcher}
RUN=${GLASS_LAUNCHER_RUN:-/run/glass-launcher}
REBOOT=${GLASS_REBOOT:-reboot -f}
KILL=${GLASS_KILL:-kill -KILL}
ALLOW_VOLATILE=${GLASS_GPU_ALLOW_VOLATILE_STATE:-0}
POLL_S=${GLASS_SUPERVISOR_POLL_S:-5}
KILL_POLLS=${GLASS_SUPERVISOR_KILL_POLLS:-6}   # polls of POLL_S after the kill: SGX can take seconds to let go
LOG_FALLBACK=${GLASS_LOG_FALLBACK:-/dev/kmsg}
REBOOT_WAIT_S=${GLASS_SUPERVISOR_REBOOT_WAIT_S:-15}
BOUND_S=${GLASS_SUPERVISOR_BOUND_S:-30}
WEDGE_CLEAR_POLLS=${GLASS_SUPERVISOR_WEDGE_CLEAR_POLLS:-720}   # an hour of beats

CTL=$SYSTEM/vendor/bin/pvrsrvctl_SGX540_120
LINKER=$SYSTEM/bin/linker
GRALLOC=$SYSTEM/vendor/lib/hw/gralloc.omap4430.so
BEAT=$RUN/beat
HUNG=$RUN/hung
UP=$RUN/gpu-up
OWNER=$RUN/launcher-pid
STARTS=$RUN/gpu-starts
TOOL_OUT=$RUN/tool.out
TOOL_PID=$RUN/tool.pid
COUNTED=$RUN/counted     # the counter's value before the open attempt, to put back on a clean stop
LOG_FIFO=$RUN/log
MAX_FAILURES=2
MAX_WEDGES=2
MAX_STARTS=5
STALE_POLLS=4   # 20 s of silence
GOOD_POLLS=4    # 20 s of beats: the GPU works

log() { echo "OMGB-GPU: $*" >&2; }

child=
watcher=
timer=
wake=

count() {
    _c=$(cat "$STATE/$1" 2>/dev/null)
    case "$_c" in ''|*[!0-9]*) _c=0 ;; esac
    echo "$_c"
}

# False unless the value reads back: a read-only or full disk must not pass for a count.
set_count() {
    echo "$2" > "$STATE/$1.new" && mv "$STATE/$1.new" "$STATE/$1" &&
        [ "$(cat "$STATE/$1" 2>/dev/null)" = "$2" ] && sync
}

# A clean stop while an attempt is open says nothing about the GPU: the count goes back. A healthy
# start or a failure closes the attempt.
undo_attempt() {
    _n=$(cat "$COUNTED" 2>/dev/null) || return 0
    rm -f "$COUNTED"
    case "$_n" in ''|*[!0-9]*) return 0 ;; esac
    set_count gpu-failures "$_n" || log "could not take the count back"
}

kill_tool() {
    _p=$(cat "$TOOL_PID" 2>/dev/null)
    [ -n "$_p" ] && $KILL "$_p" 2>/dev/null
    rm -f "$TOOL_PID"
}

on_term() {
    for _p in "$watcher" "$timer" "$child"; do
        [ -n "$_p" ] && kill "$_p" 2>/dev/null
    done
    rm -f "$OWNER"
    kill_tool
    undo_attempt
    exit 143
}
trap on_term TERM INT
trap 'wake=1' USR1   # interrupts a wait: see bounded and watch_beat
trap '' PIPE         # whoever reads the log may die; the UI must not die with it

# Runs a command in the background and returns its status, or 124 after $1 seconds. It does not
# wait for what it kills, so a process in uninterruptible sleep cannot hold it.
bounded() {
    _limit=$1
    shift
    wake=
    "$@" &
    child=$!
    (sleep "$_limit"; kill -USR1 $$) &
    timer=$!
    wait "$child"
    _rc=$?
    kill "$timer" 2>/dev/null
    timer=
    if [ -n "$wake" ]; then
        log "$1 is still running after ${_limit}s: leaving it behind"
        $KILL "$child" 2>/dev/null
        kill_tool
        _rc=124
    fi
    child=
    return "$_rc"
}

blobs_present() {
    for _f in "$LINKER" "$CTL" "$GRALLOC" "$GLES_LAUNCHER" "$FBWIN" "$MODULE"; do
        if [ ! -e "$_f" ]; then
            log "no $_f: software renderer"
            return 1
        fi
    done
}

# The type of the filesystem $STATE is on; the last mount of the longest matching path wins.
state_fstype() {
    _d=$(cd "$STATE" 2>/dev/null && pwd -P) || return 1
    awk -v d="$_d" '
        { m = $2; if ((m == "/" || d == m || index(d, m "/") == 1) && length(m) >= best) { best = length(m); t = $3 } }
        END { if (t != "") print t }' "$PROC/mounts" 2>/dev/null
}

state_persistent() {
    _t=$(state_fstype)
    case "$_t" in
        ext2|ext3|ext4|f2fs|btrfs|xfs|vfat|jffs2|ubifs|yaffs|yaffs2) return 0 ;;
    esac
    if [ "$ALLOW_VOLATILE" = 1 ]; then
        log "WARNING: $STATE is on ${_t:-an unknown filesystem}, so a GPU kernel crash is a boot loop; allowed by GLASS_GPU_ALLOW_VOLATILE_STATE"
        return 0
    fi
    log "$STATE is on ${_t:-an unknown filesystem}, which does not survive a reboot: software renderer"
    return 1
}

write_marker() {
    true > "$STATE/no-gpu"
    sync
}

# Counts the attempt before it starts; false when the limit is reached or it cannot be counted.
attempt() {
    _n=$(count gpu-failures)
    _w=$(count gpu-wedges)
    if [ "$_n" -ge "$MAX_FAILURES" ]; then
        log "$_n GPU failures in a row: writing $STATE/no-gpu, software renderer from now on"
        write_marker
        return 1
    fi
    if [ "$_w" -ge "$MAX_WEDGES" ]; then
        log "$_w reboots for a stuck GPU driver: writing $STATE/no-gpu, software renderer from now on"
        write_marker
        return 1
    fi
    # Written first, so that a stop at any point after this can put the count back.
    echo "$_n" > "$COUNTED"
    if ! set_count gpu-failures $((_n + 1)); then
        rm -f "$COUNTED"
        log "cannot write $STATE/gpu-failures, so a crash could not be counted: software renderer"
        return 1
    fi
}

may_try_gpu() {
    mkdir -p "$STATE" 2>/dev/null
    if [ -e "$STATE/no-gpu" ]; then
        log "$STATE/no-gpu is set: software renderer. $0 --clear-gpu-state tries the GPU again"
        return 1
    fi
    blobs_present && state_persistent && attempt
}

# An empty, valid bionic property area: 128 KiB, bytes used 32, serial 0, magic, version.
# Without one, property_get faults in the stock libraries.
property_area() {
    _p=$DEV/__properties__
    dd if=/dev/zero of="$_p" bs=1024 count=128 2>/dev/null || return 1
    printf '\040\000\000\000\000\000\000\000PROP\253\320\156\374' |
        dd of="$_p" conv=notrunc 2>/dev/null || return 1
    chown 0:0 "$_p" 2>/dev/null
    chmod 0444 "$_p"
}

gpu_env() {
    LD_LIBRARY_PATH=$SYSTEM/lib:$SYSTEM/vendor/lib:$SYSTEM/vendor/lib/egl:$SYSTEM/vendor/lib/hw:$(dirname "$FBWIN")
    export LD_LIBRARY_PATH
}

# Output goes to a file, not a $(...): a stuck process, or a child it left, would hold the pipe.
# The pid is for bounded, which cannot reach this process otherwise.
run_tool() {
    _what=$1
    shift
    "$@" > "$TOOL_OUT" 2>&1 &
    _tool=$!
    echo "$_tool" > "$TOOL_PID"
    if wait "$_tool"; then
        rm -f "$TOOL_PID"
        return 0
    fi
    rm -f "$TOOL_PID"
    log "$_what failed: $(cat "$TOOL_OUT" 2>/dev/null)"
    return 1
}

start_services() {
    gpu_env
    exec "$CTL" --start --no-module
}

# Module, device nodes and GPU services, once per boot. Runs under bounded.
gpu_up() {
    [ -e "$UP" ] && return 0
    mkdir -p "$DEV/graphics"
    ln -sf "$FB" "$DEV/graphics/fb0" || return 1
    property_area || { log "property area failed"; return 1; }
    if ! grep -q '^pvrsrvkm' "$PROC/modules" 2>/dev/null; then
        run_tool insmod insmod "$MODULE" || return 1
    fi
    _maj=$(awk '$2 == "pvrsrvkm" { print $1; exit }' "$PROC/devices" 2>/dev/null)
    if [ -z "$_maj" ]; then
        log "pvrsrvkm is not in $PROC/devices"
        return 1
    fi
    if [ ! -e "$DEV/pvrsrvkm" ] && ! mknod -m 666 "$DEV/pvrsrvkm" c "$_maj" 0; then
        log "mknod pvrsrvkm failed"
        return 1
    fi
    run_tool pvrsrvctl start_services || return 1
    # $RUN holds the watchdog's files: without it no launcher can be watched.
    true > "$UP" || { log "cannot write $UP"; return 1; }
    log "GPU services up"
}

# True while the launcher $1 lives and is still the one this supervisor watches.
still_stuck() {
    kill -0 "$1" 2>/dev/null && [ "$(cat "$OWNER" 2>/dev/null)" = "$1" ]
}

# Everything here may block on the stuck driver, so it runs apart from the fallback in watch_beat.
wedge_reboot() {
    _w=$(count gpu-wedges)
    # If this fails the next boot cannot write its own counter either, and refuses the GPU.
    set_count gpu-wedges $((_w + 1)) || log "could not count the reboot"
    sync
    if ! still_stuck "$1"; then
        log "the GPU launcher went after all: no reboot"
        if [ "$_w" = 0 ]; then rm -f "$STATE/gpu-wedges"; else set_count gpu-wedges "$_w"; fi
        return 0
    fi
    $REBOOT
}

# Runs in the background next to the launcher. Timers stop in deep sleep here and in the
# launcher alike, so polls count awake time and sleep is never taken for a hang.
watch_beat() {
    trap - TERM INT
    _pid=$1
    _last=
    _stale=0
    _good=0
    while :; do
        sleep "$POLL_S"
        # A supervisor killed outright, or a newer launcher: this PID is no longer ours to act on.
        kill -0 "$$" 2>/dev/null || return 0
        [ "$(cat "$OWNER" 2>/dev/null)" = "$_pid" ] || return 0
        _cur=$(cat "$BEAT" 2>/dev/null)
        if [ -n "$_cur" ] && [ "$_cur" != "$_last" ]; then
            _last=$_cur
            _stale=0
            _good=$((_good + 1))
            if [ "$_good" = "$GOOD_POLLS" ]; then
                set_count gpu-failures 0 || log "cannot reset $STATE/gpu-failures"
                rm -f "$COUNTED"
                log "GPU launcher healthy"
            fi
            if [ "$_good" = "$WEDGE_CLEAR_POLLS" ] && [ -e "$STATE/gpu-wedges" ]; then
                rm -f "$STATE/gpu-wedges"
                sync
                log "GPU launcher healthy for a long time: stuck-driver reboots forgotten"
            fi
            continue
        fi
        _stale=$((_stale + 1))
        [ "$_stale" -lt "$STALE_POLLS" ] && continue
        log "GPU launcher silent for $STALE_POLLS polls of ${POLL_S}s: killing it"
        true > "$HUNG"
        $KILL "$_pid" 2>/dev/null
        # Once it is gone the supervisor reaps it and stops this watcher.
        _k=0
        while [ "$_k" -lt "$KILL_POLLS" ]; do
            sleep "$POLL_S"
            still_stuck "$_pid" || return 0
            _k=$((_k + 1))
        done
        log "GPU launcher cannot be killed, the driver is stuck: rebooting"
        wedge_reboot "$_pid" &
        sleep "$REBOOT_WAIT_S"
        still_stuck "$_pid" || return 0
        log "no reboot after ${REBOOT_WAIT_S}s: going on without it"
        kill -USR1 "$$" 2>/dev/null
        return 0
    done
}

# True if the launcher may be tried again, false if it hung.
run_gles() {
    rm -f "$BEAT" "$HUNG"
    _s=$(cat "$STARTS" 2>/dev/null)
    case "$_s" in ''|*[!0-9]*) _s=0 ;; esac
    echo $((_s + 1)) > "$STARTS"
    log "starting the GPU launcher"
    (gpu_env; exec "$GLES_LAUNCHER" --display fbwin "$FBWIN" --heartbeat "$BEAT" "$@") &
    child=$!
    echo "$child" > "$OWNER"
    watch_beat "$child" &
    watcher=$!
    wake=
    wait "$child"
    _rc=$?
    _stuck=$wake
    kill "$watcher" 2>/dev/null   # no wait: nothing here may block the fallback
    rm -f "$OWNER"
    child=
    watcher=
    rm -f "$COUNTED"
    if [ -n "$_stuck" ]; then
        log "leaving the stuck GPU launcher behind"
        return 1
    fi
    log "GPU launcher ended, status $_rc"
    [ ! -e "$HUNG" ]
}

restart_allowed() {
    _s=$(cat "$STARTS" 2>/dev/null)
    case "$_s" in ''|*[!0-9]*) _s=$MAX_STARTS ;; esac
    [ "$_s" -lt "$MAX_STARTS" ] && return 0
    log "GPU launcher started $_s times this boot: software renderer until the next boot"
    return 1
}

run_software() {
    while :; do
        log "starting the software launcher"
        "$SOFT_LAUNCHER" --renderer software --display fb "$FB" "$@" &
        child=$!
        wait "$child"
        _rc=$?
        child=
        log "software launcher ended, status $_rc"
        sleep 1
    done
}

if [ "${1:-}" = --clear-gpu-state ]; then
    rm -f "$STATE/no-gpu" "$STATE/gpu-failures" "$STATE/gpu-wedges" || exit 1
    sync
    log "GPU state cleared: the next start tries the GPU"
    exit 0
fi

# Copies the launchers' output to the init, or to the fallback once that fails. It only ends when
# every writer is gone.
relay() {
    trap - TERM INT
    _to=
    while IFS= read -r _l; do
        if [ -z "$_to" ]; then
            printf '%s\n' "$_l" >&2 && continue
            _to=$LOG_FALLBACK
            [ -w "$_to" ] || _to=/dev/null
            printf 'OMGB-GPU: the log reader is gone: output goes to %s\n' "$_to" >> "$_to"
        fi
        printf '%s\n' "$_l" >> "$_to" 2>/dev/null
    done
}

mkdir -p "$RUN"
rm -f "$COUNTED" "$LOG_FIFO"
if mkfifo "$LOG_FIFO" 2>/dev/null; then
    relay < "$LOG_FIFO" &
    # Read-write: the open cannot block, and while this shell lives the relay sees no end of file.
    exec 1<> "$LOG_FIFO" 2>&1
else
    log "no log relay: a dead log reader would stop the launchers"
fi
if bounded "$BOUND_S" may_try_gpu; then
    if bounded "$BOUND_S" gpu_up; then
        while run_gles "$@" && restart_allowed && bounded "$BOUND_S" attempt; do
            sleep 1
        done
    else
        log "GPU bring-up failed: software renderer"
    fi
    rm -f "$COUNTED"
fi
run_software "$@"
