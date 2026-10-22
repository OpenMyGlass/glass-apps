# SPDX-License-Identifier: Apache-2.0
# Product makefile for the OpenMyGlass launcher. In the device tree:
#   $(call inherit-product, vendor/openmyglass/glass-apps/glass-apps.mk)
GLASS_APPS_DIR := vendor/openmyglass/glass-apps

$(foreach f,GlassLauncher.apk lib/libglass_launcher_jni.so,\
  $(if $(wildcard $(GLASS_APPS_DIR)/$(f)),,\
    $(error no $(GLASS_APPS_DIR)/$(f): run glass-apps' tools/android-dist.sh $(GLASS_APPS_DIR) or unpack its CI artifact glass-apps-android-dist there)))

PRODUCT_PACKAGES += \
    GlassLauncher \
    libglass_launcher_jni
