#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Builds the Android launcher (both tracks: one API 19 build for Android 4.4 and 6.0) into a
# self-contained directory for an AOSP tree:
#   tools/android-dist.sh <out-dir>        (normally <aosp>/vendor/openmyglass/glass-apps)
# Writes <out-dir>/{Android.mk,glass-apps.mk,GlassLauncher.apk,lib/libglass_launcher_jni.so,
# SOURCE,SHA256SUMS}. AOSP re-signs the APK with the platform key (LOCAL_CERTIFICATE).
#
# Needs: cargo with the armv7-linux-androideabi target; ANDROID_NDK_HOME = NDK r25c (the
# last NDK that targets API 19); ANDROID_HOME with platforms;android-19 and build-tools
# $BUILD_TOOLS (default 34.0.0); a JDK (javac, keytool).
set -eu

if [ $# -ne 1 ]; then
    echo "usage: $0 <out-dir>" >&2
    exit 2
fi
here=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$1"
out=$(cd "$1" && pwd)
API=19
BUILD_TOOLS=${BUILD_TOOLS:-34.0.0}
NDK_REVISION=25.2.9519653   # r25c

: "${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to NDK r25c}"
: "${ANDROID_HOME:?set ANDROID_HOME to the Android SDK}"
rev=$(sed -n 's/^Pkg.Revision *= *//p' "$ANDROID_NDK_HOME/source.properties")
if [ "$rev" != "$NDK_REVISION" ]; then
    echo "error: NDK $rev at $ANDROID_NDK_HOME, need r25c ($NDK_REVISION)" >&2
    exit 1
fi
jar=$ANDROID_HOME/platforms/android-$API/android.jar
bt=$ANDROID_HOME/build-tools/$BUILD_TOOLS
for f in "$jar" "$bt/aapt2" "$bt/d8" "$bt/zipalign" "$bt/apksigner"; do
    [ -e "$f" ] || { echo "error: missing $f" >&2; exit 1; }
done
host=$(uname -s | tr '[:upper:]' '[:lower:]')-x86_64
bin=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$host/bin

# Native library: API 19 so one build loads on 4.4 and 6.0; every symbol must resolve
# against API 19's libraries (--no-undefined); a soname for Android 6's linker.
export CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER="$bin/armv7a-linux-androideabi$API-clang"
export CC_armv7_linux_androideabi="$bin/armv7a-linux-androideabi$API-clang"
export AR_armv7_linux_androideabi="$bin/llvm-ar"
export CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_RUSTFLAGS="-C link-arg=-Wl,--no-undefined -C link-arg=-Wl,-soname,libglass_launcher_jni.so"
(cd "$here" && cargo build --release --locked --target armv7-linux-androideabi -p glass-android)
mkdir -p "$out/lib"
"$bin/llvm-strip" --strip-unneeded -o "$out/lib/libglass_launcher_jni.so" \
    "$here/target/armv7-linux-androideabi/release/libglass_launcher_jni.so"

# The APK: Java 8 bytecode against android-19, dex for API 19, no resources but the manifest.
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
src=$here/android/launcher
mkdir -p "$work/classes" "$work/dex"
# shellcheck disable=SC2046
javac -source 8 -target 8 -Xlint:-options -encoding UTF-8 -bootclasspath "$jar" \
    -d "$work/classes" $(find "$src/src" -name '*.java')
# shellcheck disable=SC2046
"$bt/d8" --release --min-api $API --lib "$jar" --output "$work/dex" \
    $(find "$work/classes" -name '*.class')
"$bt/aapt2" link --manifest "$src/AndroidManifest.xml" -I "$jar" \
    --min-sdk-version $API --target-sdk-version 22 -o "$work/unsigned.apk"
(cd "$work/dex" && zip -q -X "$work/unsigned.apk" classes.dex)
"$bt/zipalign" -f -p 4 "$work/unsigned.apk" "$work/aligned.apk"
# A throwaway key: the build system signs the APK again with the platform key.
keytool -genkeypair -keystore "$work/key.jks" -storepass android -keypass android \
    -alias dist -keyalg RSA -keysize 2048 -validity 1 -dname "CN=glass-apps dist" >/dev/null 2>&1
"$bt/apksigner" sign --ks "$work/key.jks" --ks-pass pass:android --min-sdk-version $API \
    --out "$out/GlassLauncher.apk" "$work/aligned.apk"

cp "$here/android/dist/Android.mk" "$here/android/dist/glass-apps.mk" "$out/"
commit=$(git -C "$here" rev-parse HEAD 2>/dev/null || echo unknown)
printf 'https://github.com/OpenMyGlass/glass-apps %s NDK %s API %s build-tools %s\n' \
    "$commit" "$rev" "$API" "$BUILD_TOOLS" > "$out/SOURCE"
(cd "$out" && find . -type f ! -name SHA256SUMS | sort | xargs sha256sum > SHA256SUMS 2>/dev/null \
    || find . -type f ! -name SHA256SUMS | sort | xargs shasum -a 256 > SHA256SUMS)
echo "android dist: $out"
