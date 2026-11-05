#!/bin/sh
# Copies the stock GPU libraries Model B needs from the owner's own Glass or its factory image,
# and checks each one against native/fbwin/gpu-blobs.sha256. These files belong to Google and
# Imagination Technologies: the result must stay on the owner's machine and Glass. Never commit
# it, publish it, or upload an image that contains it.
#
# Usage: tools/extract-gpu-blobs.sh SOURCE OUT_DIR
#   SOURCE  adb[:SERIAL]    a Glass running stock Android (XE24)
#           FILE            the factory system.img, sparse or raw (needs debugfs, python3)
#           DIRECTORY       a copy of the system partition
#   OUT_DIR the root of the tree to add to the image; must be outside any git work tree
#
# Exit status: 0 all files present and verified, 1 otherwise, 2 bad usage.

set -u

[ $# = 2 ] || { sed -n 2,13p "$0" >&2; exit 2; }
source=$1
out=$2
here=$(cd "$(dirname "$0")" && pwd) || exit 1
sums=$here/../native/fbwin/gpu-blobs.sha256

die() { echo "extract-gpu-blobs: $*" >&2; exit 1; }

[ -r "$sums" ] || die "cannot read $sums"
mkdir -p "$out" || die "cannot create $out"
out=$(cd "$out" && pwd) || exit 1
if git -C "$out" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    die "$out is inside a git work tree: these files must never be committed"
fi

work=$(mktemp -d) || exit 1
trap 'rm -rf "$work"' EXIT

# Fetches system/PATH of the source to the file $2. Failures show up at the hash check.
case $source in
    adb|adb:*)
        serial=${source#adb}
        serial=${serial#:}
        fetch() { adb ${serial:+-s "$serial"} pull "/$1" "$2" >/dev/null 2>&1; }
        ;;
    *)
        if [ -d "$source" ]; then
            fetch() { cp "$source/${1#system/}" "$2" 2>/dev/null; }
        elif [ -f "$source" ]; then
            command -v debugfs >/dev/null || die "debugfs (e2fsprogs) is needed to read an image"
            image=$source
            if [ "$(od -An -tx4 -N4 "$source" | tr -d ' ')" = ed26ff3a ]; then
                image=$work/system.raw
                python3 -I - "$source" "$image" <<'PY' || die "cannot read the sparse image"
import struct, sys
with open(sys.argv[1], "rb") as src, open(sys.argv[2], "wb") as dst:
    magic, major, _, fh, ch, blk, _, chunks, _ = struct.unpack("<IHHHHIIII", src.read(28))
    assert magic == 0xED26FF3A and major == 1 and blk
    src.seek(fh)
    for _ in range(chunks):
        kind, _, blocks, total = struct.unpack("<HHII", src.read(12))
        src.seek(ch - 12, 1)
        size = blocks * blk
        if kind == 0xCAC1:
            dst.write(src.read(size))
        elif kind == 0xCAC2:
            fill = src.read(4)
            dst.write(fill * (size // 4))
        elif kind == 0xCAC3:
            dst.seek(size, 1)
        elif kind == 0xCAC4:
            src.seek(total - ch, 1)
        else:
            raise SystemExit("unknown chunk")
    dst.truncate()
PY
            fi
            fetch() { debugfs -R "dump /${1#system/} $2" "$image" >/dev/null 2>&1; }
        else
            die "no such source: $source"
        fi
        ;;
esac

bad=0
count=0
while read -r sum path; do
    case "$sum" in ''|'#'*) continue ;; esac
    dest=$out/$path
    mkdir -p "$(dirname "$dest")" || die "cannot create the directory of $dest"
    rm -f "$dest"
    fetch "$path" "$work/file"
    got=$(sha256sum "$work/file" 2>/dev/null | cut -d' ' -f1)
    if [ "$got" != "$sum" ]; then
        echo "extract-gpu-blobs: $path: ${got:-missing}, expected $sum" >&2
        bad=$((bad + 1))
        continue
    fi
    cp "$work/file" "$dest" || die "cannot write $dest"
    rm -f "$work/file"
    case $path in
        system/bin/*|system/vendor/bin/*) chmod 755 "$dest" ;;
        *) chmod 644 "$dest" ;;
    esac
    count=$((count + 1))
done < "$sums"
[ "$bad" = 0 ] || die "$bad files missing or different, is this Glass on XE24? $count verified"

# Where the stock libraries look: /vendor for hw modules and EGL, and with no ro.hardware
# hw_get_module only tries gralloc.default.
mkdir -p "$out/vendor" "$out/etc"
ln -sfn /system/vendor/lib "$out/vendor/lib"
ln -sfn /system/vendor/bin "$out/vendor/bin"
ln -sf gralloc.omap4430.so "$out/system/vendor/lib/hw/gralloc.default.so"
cp "$out/system/etc/powervr.ini" "$out/etc/powervr.ini" || die "cannot copy powervr.ini"
echo "extract-gpu-blobs: $count files verified in $out. They are not ours: keep them local."
