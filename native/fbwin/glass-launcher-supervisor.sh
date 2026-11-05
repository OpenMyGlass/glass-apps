#!/bin/sh
# Model B: starts the launcher on the GPU when that can work, on the software renderer when it
# cannot, and never leaves the Glass without a UI. Logs to stderr; the init sends it to kmsg.
#
# Usage: glass-launcher-supervisor.sh [launcher arguments...]
#
# - No blobs (the fully open configuration), or a no-gpu marker: software at once.
# - Every GPU attempt counts as a failure until the launcher has beaten for 20 s, also across
#   reboots, so a kernel crash counts too. Two in a row write the marker. Reinstalling removes it.
# - A launcher that stops beating for 20 s of awake time is killed and this boot goes on in
#   software. If it cannot be killed the driver is stuck and only a reboot brings the UI back.
#
# Every path can be overridden from the environment, which is how the tests run this on a host.

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
POLL_S=${GLASS_SUPERVISOR_POLL_S:-5}
KILL_WAIT_S=${GLASS_SUPERVISOR_KILL_WAIT_S:-5}

CTL=$SYSTEM/vendor/bin/pvrsrvctl_SGX540_120
LINKER=$SYSTEM/bin/linker
GRALLOC=$SYSTEM/vendor/lib/hw/gralloc.omap4430.so
BEAT=$RUN/beat
HUNG=$RUN/hung
UP=$RUN/gpu-up
MAX_FAILURES=2
STALE_POLLS=4   # 20 s of silence
GOOD_POLLS=4    # 20 s of beats: the GPU works

log() { echo "OMGB-GPU: $*" >&2; }

child=
trap '[ -n "$child" ] && kill "$child" 2>/dev/null; exit 143' TERM INT

blobs_present() {
    for _f in "$LINKER" "$CTL" "$GRALLOC" "$GLES_LAUNCHER" "$FBWIN" "$MODULE"; do
        if [ ! -e "$_f" ]; then
            log "no $_f: software renderer"
            return 1
        fi
    done
}

set_failures() {
    echo "$1" > "$STATE/gpu-failures.new" && mv "$STATE/gpu-failures.new" "$STATE/gpu-failures"
    sync
}

# Counts the attempt before it starts; false when the limit is reached.
attempt() {
    _n=$(cat "$STATE/gpu-failures" 2>/dev/null)
    case "$_n" in ''|*[!0-9]*) _n=0 ;; esac
    if [ "$_n" -ge "$MAX_FAILURES" ]; then
        log "$_n GPU failures in a row: writing $STATE/no-gpu, software renderer from now on"
        : > "$STATE/no-gpu"
        sync
        return 1
    fi
    set_failures $((_n + 1))
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

# Module, device nodes and GPU services, once per boot.
gpu_up() {
    [ -e "$UP" ] && return 0
    mkdir -p "$DEV/graphics"
    ln -sf "$FB" "$DEV/graphics/fb0" || return 1
    property_area || { log "property area failed"; return 1; }
    if ! grep -q '^pvrsrvkm' "$PROC/modules" 2>/dev/null; then
        if ! _o=$(insmod "$MODULE" 2>&1); then
            log "insmod failed: $_o"
            return 1
        fi
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
    if ! _o=$(gpu_env; timeout -s KILL 30 "$CTL" --start --no-module 2>&1); then
        log "pvrsrvctl failed: $_o"
        return 1
    fi
    : > "$UP"
    log "GPU services up"
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
        _cur=$(cat "$BEAT" 2>/dev/null)
        if [ -n "$_cur" ] && [ "$_cur" != "$_last" ]; then
            _last=$_cur
            _stale=0
            _good=$((_good + 1))
            if [ "$_good" = "$GOOD_POLLS" ]; then
                set_failures 0
                log "GPU launcher healthy"
            fi
            continue
        fi
        _stale=$((_stale + 1))
        [ "$_stale" -lt "$STALE_POLLS" ] && continue
        log "GPU launcher silent for $STALE_POLLS polls of ${POLL_S}s: killing it"
        : > "$HUNG"
        kill -KILL "$_pid" 2>/dev/null
        sleep "$KILL_WAIT_S"
        if kill -0 "$_pid" 2>/dev/null; then
            log "GPU launcher cannot be killed, the driver is stuck: rebooting"
            sync
            $REBOOT
        fi
        return 0
    done
}

# True if the launcher may be tried again, false if it hung.
run_gles() {
    rm -f "$BEAT" "$HUNG"
    log "starting the GPU launcher"
    (gpu_env; exec "$GLES_LAUNCHER" --display fbwin "$FBWIN" --heartbeat "$BEAT" "$@") &
    child=$!
    watch_beat "$child" &
    _watcher=$!
    wait "$child"
    _rc=$?
    child=
    kill "$_watcher" 2>/dev/null
    wait "$_watcher" 2>/dev/null
    log "GPU launcher ended, status $_rc"
    [ ! -e "$HUNG" ]
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

mkdir -p "$STATE" "$RUN"
if [ -e "$STATE/no-gpu" ]; then
    log "$STATE/no-gpu is set: software renderer"
elif blobs_present && attempt; then
    if gpu_up; then
        while run_gles "$@" && attempt; do
            sleep 1
        done
    else
        log "GPU bring-up failed: software renderer"
    fi
fi
run_software "$@"
