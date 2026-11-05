#!/bin/sh
# Host test of glass-launcher-supervisor.sh with fake launchers and fake GPU tools.
# Usage: native/fbwin/test_supervisor.sh
here=$(cd "$(dirname "$0")" && pwd)
SUP=$here/glass-launcher-supervisor.sh
T=$(mktemp -d) || exit 1
trap 'pkill -KILL -f "$T/" 2>/dev/null; rm -rf "$T"' EXIT
fails=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; sed 's/^/     /' "$T/log" 2>/dev/null; fails=$((fails + 1)); }

mkdir -p "$T/fake"
for t in insmod mknod; do
    printf '#!/bin/sh\necho "%s $*" >> "%s/calls"\n' "$t" "$T" > "$T/fake/$t"
    chmod 755 "$T/fake/$t"
done

# A fresh world. $1: what the GPU launcher does.
world() {
    W=$T/w; pkill -KILL -f "$T/" 2>/dev/null; rm -rf "$W"; rm -f "$T/calls" "$T/log"
    mkdir -p "$W/system/bin" "$W/system/vendor/bin" "$W/system/vendor/lib/hw" "$W/lib" "$W/dev" "$W/proc" "$W/state" "$W/run"
    : > "$W/system/bin/linker"; : > "$W/system/vendor/lib/hw/gralloc.omap4430.so"; : > "$W/lib/libglass_fbwin.so"; : > "$W/lib/module.ko"; : > "$W/dev/fb0"
    : > "$W/proc/modules"; echo '242 pvrsrvkm' > "$W/proc/devices"
    printf '#!/bin/sh\necho "ctl $*" >> "%s/calls"\necho "LD=$LD_LIBRARY_PATH" >> "%s/calls"\n' "$T" "$T" > "$W/system/vendor/bin/pvrsrvctl_SGX540_120"
    printf '#!/bin/sh\necho "soft $*" >> "%s/calls"\necho soft > "%s/started"\nexec sleep 1000\n' "$T" "$W" > "$W/soft"
    case $1 in
        beat) body='i=0; while :; do i=$((i+1)); echo $i > "$BEATF"; sleep 0.05; done' ;;
        crash) body='exit 3' ;;
        silent) body='exec sleep 1000' ;;
        crash-after-beats) body='i=0; while [ $i -lt 40 ]; do i=$((i+1)); echo $i > "$BEATF"; sleep 0.05; done; exit 3' ;;
    esac
    cat > "$W/gles" <<G
#!/bin/sh
echo "gles \$* LD=\$LD_LIBRARY_PATH" >> "$T/calls"
while [ \$# -gt 0 ]; do [ "\$1" = --heartbeat ] && BEATF=\$2; shift; done
$body
G
    chmod 755 "$W/system/vendor/bin/pvrsrvctl_SGX540_120" "$W/soft" "$W/gles"
}

sup() { # seconds, then TERM
    env PATH="$T/fake:$PATH" GLASS_SYSTEM="$W/system" GLASS_GPU_MODULE="$W/lib/module.ko" \
        GLASS_GLES_LAUNCHER="$W/gles" GLASS_SOFT_LAUNCHER="$W/soft" GLASS_FBWIN="$W/lib/libglass_fbwin.so" \
        GLASS_FB="$W/dev/fb0" GLASS_DEV="$W/dev" GLASS_PROC="$W/proc" GLASS_LAUNCHER_STATE="$W/state" \
        GLASS_LAUNCHER_RUN="$W/run" GLASS_REBOOT="touch $W/rebooted" GLASS_SUPERVISOR_POLL_S=0.2 \
        GLASS_SUPERVISOR_KILL_WAIT_S=0.2 ${SHELL_UNDER_TEST:-sh} "$SUP" --glassd sock >> "$T/log" 2>&1 &
    _p=$!
    sleep "$1"
    kill "$_p" 2>/dev/null
    wait "$_p" 2>/dev/null
    pkill -KILL -f "$W/" 2>/dev/null
    sleep 0.1
}
called() { grep -q -- "$1" "$T/calls" 2>/dev/null; }
count() { grep -c -- "$1" "$T/calls" 2>/dev/null; }

world beat; rm "$W/system/vendor/bin/pvrsrvctl_SGX540_120"; sup 1
if called '^soft' && ! called '^gles' && ! called '^insmod' && grep -q 'software renderer' "$T/log"; then ok "no blobs: software, nothing loaded"; else bad "no blobs"; fi

world beat; : > "$W/state/no-gpu"; sup 1
if called '^soft' && ! called '^gles' && grep -q 'no-gpu is set' "$T/log"; then ok "marker: software"; else bad "marker"; fi

world beat; sup 2.5
if called '^gles --display fbwin .*libglass_fbwin.so --heartbeat .*--glassd sock' && ! called '^soft' && called "LD=$W/system/lib:" \
    && called '^ctl --start --no-module' && called '^insmod' && called '^mknod -m 666 .*pvrsrvkm c 242 0' \
    && [ "$(cat "$W/state/gpu-failures")" = 0 ] && grep -q healthy "$T/log" && [ -L "$W/dev/graphics/fb0" ]; then
    ok "healthy GPU launcher: counter back to 0"; else bad "healthy"; fi
python3 - "$W/dev/__properties__" <<'P' && ok "property area bytes" || bad "property area bytes"
import struct,sys
d=open(sys.argv[1],'rb').read()
e=bytearray(128*1024); struct.pack_into('<4I',e,0,32,0,0x504f5250,0xfc6ed0ab)
sys.exit(0 if d==bytes(e) else 1)
P

world crash; sup 2
if [ "$(count '^gles')" = 2 ] && called '^soft' && [ -e "$W/state/no-gpu" ]; then ok "two crashes: marker, software"; else bad "two crashes ($(count '^gles'))"; fi

world crash-after-beats; sup 6
if [ "$(count '^gles')" -ge 2 ] && ! [ -e "$W/state/no-gpu" ] && ! called '^soft'; then ok "crashes after running well: retried, no marker"; else bad "crash after beats"; fi

world silent; sup 2.5
if [ "$(count '^gles')" = 1 ] && called '^soft' && ! [ -e "$W/state/no-gpu" ] && [ "$(cat "$W/state/gpu-failures")" = 1 ] && ! [ -e "$W/rebooted" ]; then
    ok "hang: killed, software, one failure"; else bad "hang"; fi

world silent; echo 1 > "$W/state/gpu-failures"; sup 2.5
if [ "$(cat "$W/state/gpu-failures")" = 2 ] && called '^soft'; then ok "second bad boot: two failures"; else bad "second bad boot"; fi
rm -f "$T/calls"; sup 1
if [ -e "$W/state/no-gpu" ] && ! called '^gles' && called '^soft'; then ok "third boot: marker, GPU not tried"; else bad "third boot"; fi

world crash; rm -f "$W/run/gpu-up"; echo 'junk' > "$W/state/gpu-failures"; sup 1
if [ "$(count '^gles')" -ge 1 ]; then ok "unreadable counter counts as 0"; else bad "junk counter"; fi

world beat; printf '#!/bin/sh\necho no >&2; exit 1\n' > "$W/system/vendor/bin/pvrsrvctl_SGX540_120"; sup 1
if called '^soft' && ! called '^gles' && grep -q 'pvrsrvctl failed: no' "$T/log"; then ok "pvrsrvctl fails: software"; else bad "pvrsrvctl fails"; fi

world beat; echo 'pvrsrvkm 1 0' > "$W/proc/modules"; sup 1
if ! called '^insmod' && called '^gles'; then ok "module already loaded"; else bad "module loaded"; fi

world crash; printf '#!/bin/sh\nexit 2\n' > "$W/soft"; sup 4.5
if [ "$(grep -c 'starting the software launcher' "$T/log")" -ge 2 ]; then ok "software launcher respawns"; else bad "respawn"; fi

[ "$fails" = 0 ] && echo "supervisor tests passed" || { echo "$fails failed"; exit 1; }
