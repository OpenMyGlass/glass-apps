# SPDX-License-Identifier: Apache-2.0
# The OpenMyGlass launcher, prebuilt by glass-apps' tools/android-dist.sh (or its CI
# artifact glass-apps-android-dist) into vendor/openmyglass/glass-apps/. Works on the
# Android 4.4.4 (Track A) and 6.0.1 (Track B) build systems.

LOCAL_PATH := $(call my-dir)

# The native launcher, in /system/lib: KitKat does not extract the JNI libraries of system
# apps, so the APK loads it from the system library path.
include $(CLEAR_VARS)
LOCAL_MODULE := libglass_launcher_jni
LOCAL_MODULE_CLASS := SHARED_LIBRARIES
LOCAL_MODULE_SUFFIX := .so
LOCAL_SRC_FILES := lib/libglass_launcher_jni.so
LOCAL_MODULE_TAGS := optional
LOCAL_MULTILIB := 32
# Stripped by android-dist.sh already.
LOCAL_STRIP_MODULE := false
include $(BUILD_PREBUILT)

# The HOME activity. Re-signed with the platform key here (it holds WRITE_SECURE_SETTINGS
# and BLUETOOTH_PRIVILEGED for the setup flow); a privileged app.
include $(CLEAR_VARS)
LOCAL_MODULE := GlassLauncher
LOCAL_MODULE_CLASS := APPS
LOCAL_MODULE_SUFFIX := $(COMMON_ANDROID_PACKAGE_SUFFIX)
LOCAL_SRC_FILES := GlassLauncher.apk
LOCAL_CERTIFICATE := platform
LOCAL_PRIVILEGED_MODULE := true
LOCAL_MODULE_TAGS := optional
LOCAL_OVERRIDES_PACKAGES := Launcher2 Launcher3 Home Provision
LOCAL_REQUIRED_MODULES := libglass_launcher_jni
include $(BUILD_PREBUILT)
