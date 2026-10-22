// SPDX-License-Identifier: Apache-2.0
package org.openmyglass.launcher;

import android.app.Activity;
import android.os.Bundle;
import android.util.Log;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.WindowManager;

/**
 * The HOME activity: a full-screen SurfaceView whose Surface goes to the native launcher,
 * which draws it with EGL and GLES2 on its own render thread. Input does not come through
 * Android: the launcher reads the touchpad gestures from glassd, takes glassd's input focus
 * while this activity is resumed and gives it back when paused (api.md "Android glue").
 */
public final class HomeActivity extends Activity implements SurfaceHolder.Callback {
    private static final String TAG = "glass-launcher";
    private NativeBridge bridge;
    private AndroidPlatform platform;

    @Override
    protected void onCreate(Bundle saved) {
        super.onCreate(saved);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_FULLSCREEN);
        SurfaceView view = new SurfaceView(this);
        view.getHolder().addCallback(this);
        setContentView(view);
        final NativeBridge[] b = new NativeBridge[1];
        platform = new AndroidPlatform(this, new AndroidPlatform.Events() {
            @Override
            public void event(String line) {
                if (b[0] != null) {
                    b[0].event(line);
                }
            }
        });
        bridge = new NativeBridge(getFilesDir().getAbsolutePath(), platform);
        b[0] = bridge;
        if (!bridge.ok()) {
            Log.e(TAG, "the native launcher did not start");
        }
        platform.register();
        platform.sendState();
    }

    @Override
    protected void onResume() {
        super.onResume();
        bridge.resume();
    }

    @Override
    protected void onPause() {
        bridge.pause();
        super.onPause();
    }

    @Override
    protected void onDestroy() {
        platform.unregister();
        bridge.destroy();
        super.onDestroy();
    }

    /** HOME has nowhere to go back to; the touchpad's swipe down is handled natively. */
    @Override
    public void onBackPressed() {}

    @Override
    public void surfaceCreated(SurfaceHolder holder) {
        bridge.surfaceCreated(holder.getSurface());
    }

    @Override
    public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
        bridge.surfaceChanged(width, height);
    }

    @Override
    public void surfaceDestroyed(SurfaceHolder holder) {
        bridge.surfaceDestroyed();
    }
}
