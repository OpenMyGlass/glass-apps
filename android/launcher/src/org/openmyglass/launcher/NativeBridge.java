// SPDX-License-Identifier: Apache-2.0
package org.openmyglass.launcher;

import android.os.Handler;
import android.os.Looper;
import android.view.Surface;

/**
 * The JNI surface of libglass_launcher_jni.so (crates/glass-android/src/jni.rs). The native
 * side runs the launcher on its own render thread; requests from it arrive in
 * {@link #onNativeRequest} on that thread and are handed to the main thread.
 */
final class NativeBridge {
    static {
        System.loadLibrary("glass_launcher_jni");
    }

    interface Requests {
        /** Called on the main thread with one request line (see {@link PlatformLines}). */
        void onRequest(String line);
    }

    private final Handler main = new Handler(Looper.getMainLooper());
    private final Requests requests;
    private long handle;

    NativeBridge(String filesDir, Requests requests) {
        this.requests = requests;
        handle = nativeCreate(filesDir);
    }

    boolean ok() {
        return handle != 0;
    }

    void surfaceCreated(Surface s) {
        if (handle != 0) nativeSurfaceCreated(handle, s);
    }

    void surfaceChanged(int w, int h) {
        if (handle != 0) nativeSurfaceChanged(handle, w, h);
    }

    /** Returns once the render thread no longer uses the surface. */
    void surfaceDestroyed() {
        if (handle != 0) nativeSurfaceDestroyed(handle);
    }

    void resume() {
        if (handle != 0) nativeResume(handle);
    }

    void pause() {
        if (handle != 0) nativePause(handle);
    }

    void event(String line) {
        if (handle != 0) nativeEvent(handle, line);
    }

    void destroy() {
        if (handle != 0) {
            long h = handle;
            handle = 0;
            nativeDestroy(h);
        }
    }

    /** From the render thread: never block it, post to the main thread. */
    @SuppressWarnings("unused")
    private void onNativeRequest(final String line) {
        main.post(new Runnable() {
            @Override
            public void run() {
                requests.onRequest(line);
            }
        });
    }

    private native long nativeCreate(String filesDir);

    private native void nativeSurfaceCreated(long h, Surface surface);

    private native void nativeSurfaceChanged(long h, int width, int height);

    private native void nativeSurfaceDestroyed(long h);

    private native void nativeResume(long h);

    private native void nativePause(long h);

    private native void nativeEvent(long h, String line);

    private native void nativeDestroy(long h);
}
