#!/bin/sh
# Fails if a dependency's licence expression names any licence outside the permissive set
# this project accepts (see README, "Third-party code"). Strict on purpose: an expression
# like "MIT OR GPL-3.0" is flagged too, for a human to look at.
# Usage: tools/check_licenses.sh [cargo tree arguments...]
set -eu
allowed=' MIT Apache-2.0 LLVM-exception BSD-2-Clause BSD-3-Clause Zlib ISC 0BSD Unlicense Unicode-3.0 '
# GPU path (glass-gfx-gles): khronos-egl (MIT/Apache-2.0), glow (MIT OR Apache-2.0 OR Zlib),
# libloading (ISC).
raw="${TMPDIR:-/tmp}/glass-licences-raw.txt"
# Not in a pipeline, so that a failing `cargo tree` fails the check.
cargo tree --color never -e normal,build --prefix none --format '{p}|{l}' "$@" >"$raw"
sed 's/ (\*)$//' "$raw" | sort -u >"${TMPDIR:-/tmp}/glass-licences.txt"
status=0
while IFS='|' read -r pkg lic; do
  case "$pkg" in glass-*) continue ;; esac
  if [ -z "$lic" ]; then echo "no licence: $pkg"; status=1; continue; fi
  for id in $(echo "$lic" | tr '()/' '   ' | sed 's/ OR / /g; s/ AND / /g; s/ WITH / /g'); do
    case "$allowed" in
      *" $id "*) ;;
      *) echo "not allowed: $pkg ($lic)"; status=1 ;;
    esac
  done
done <"${TMPDIR:-/tmp}/glass-licences.txt"
[ "$status" = 0 ] && echo "licences ok: $(wc -l <"${TMPDIR:-/tmp}/glass-licences.txt" | tr -d ' ') packages"
exit "$status"
