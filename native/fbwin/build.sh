#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Builds libglass_fbwin.so and gpu_probe for the stock Android 4.4 linker: API 19, NDK r25c,
# softfp armeabi-v7a, sysv hash. Output: $1. Needs ANDROID_NDK_HOME.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$1
mkdir -p "$out"
cc=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/armv7a-linux-androideabi19-clang
common="-O2 -Wall -fPIC -Wl,--hash-style=sysv -Wl,--no-undefined"
$cc $common -shared -Wl,-soname,libglass_fbwin.so -o "$out/libglass_fbwin.so" "$here/glass_fbwin.c" -ldl
$cc $common -pie -fPIE -o "$out/gpu_probe" "$here/gpu_probe.c" -lEGL -lGLESv2 -ldl
