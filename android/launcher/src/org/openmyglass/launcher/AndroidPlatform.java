// SPDX-License-Identifier: Apache-2.0
package org.openmyglass.launcher;

import android.bluetooth.BluetoothAdapter;
import android.bluetooth.BluetoothDevice;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.net.wifi.WifiInfo;
import android.net.wifi.WifiManager;
import android.provider.Settings;
import android.util.Log;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.List;

/**
 * Carries out the native launcher's platform requests (Wi-Fi, Bluetooth, pairing, the
 * set-up flag, opening apps) and reports state changes back as event lines.
 */
final class AndroidPlatform extends BroadcastReceiver implements NativeBridge.Requests {
    private static final String TAG = "glass-launcher";
    /** BluetoothAdapter.SCAN_MODE_CONNECTABLE_DISCOVERABLE. */
    private static final int SCAN_MODE_DISCOVERABLE = 23;

    interface Events {
        void event(String line);
    }

    private final Context context;
    private final Events events;
    private final WifiManager wifi;
    private final BluetoothAdapter bt;
    /** The device of the pairing request shown, until it is answered. */
    private BluetoothDevice pending;

    AndroidPlatform(Context context, Events events) {
        this.context = context;
        this.events = events;
        this.wifi = (WifiManager) context.getSystemService(Context.WIFI_SERVICE);
        this.bt = BluetoothAdapter.getDefaultAdapter();
    }

    void register() {
        IntentFilter f = new IntentFilter();
        f.addAction(WifiManager.WIFI_STATE_CHANGED_ACTION);
        f.addAction(WifiManager.NETWORK_STATE_CHANGED_ACTION);
        f.addAction(BluetoothAdapter.ACTION_STATE_CHANGED);
        f.addAction(BluetoothAdapter.ACTION_LOCAL_NAME_CHANGED);
        f.addAction(BluetoothDevice.ACTION_BOND_STATE_CHANGED);
        context.registerReceiver(this, f);
        // Ordered: taken before the system's pairing dialog, which the touchpad cannot use.
        IntentFilter p = new IntentFilter(BluetoothDevice.ACTION_PAIRING_REQUEST);
        p.setPriority(IntentFilter.SYSTEM_HIGH_PRIORITY - 1);
        context.registerReceiver(this, p);
    }

    void unregister() {
        context.unregisterReceiver(this);
    }

    /** The current state, sent at start. */
    void sendState() {
        sendWifi();
        sendBluetooth();
        int done = Settings.Global.getInt(context.getContentResolver(),
                Settings.Global.DEVICE_PROVISIONED, 0);
        events.event(PlatformLines.provisioned(done != 0));
    }

    private void sendWifi() {
        if (wifi == null) {
            return;
        }
        boolean on = wifi.isWifiEnabled();
        WifiInfo info = on ? wifi.getConnectionInfo() : null;
        String ssid = info == null ? "" : PlatformLines.ssid(info.getSSID());
        events.event(PlatformLines.wifi(on, ssid));
    }

    private void sendBluetooth() {
        if (bt == null) {
            return;
        }
        boolean on = bt.isEnabled();
        List<String> names = new ArrayList<String>();
        if (on) {
            for (BluetoothDevice d : bt.getBondedDevices()) {
                names.add(d.getName() == null ? d.getAddress() : d.getName());
            }
            events.event(PlatformLines.btName(bt.getName()));
        }
        events.event(PlatformLines.bluetooth(on, names));
    }

    @Override
    public void onReceive(Context c, Intent intent) {
        String a = intent.getAction();
        if (BluetoothDevice.ACTION_PAIRING_REQUEST.equals(a)) {
            BluetoothDevice d = intent.getParcelableExtra(BluetoothDevice.EXTRA_DEVICE);
            int variant = intent.getIntExtra(BluetoothDevice.EXTRA_PAIRING_VARIANT, -1);
            int key = intent.getIntExtra(BluetoothDevice.EXTRA_PAIRING_KEY, -1);
            // Passkey confirmation (2) and consent (3) can be answered with a tap; others
            // (a PIN to type) are left to the system.
            if (d == null || (variant != BluetoothDevice.PAIRING_VARIANT_PASSKEY_CONFIRMATION
                    && variant != 3)) {
                return;
            }
            pending = d;
            String passkey = key >= 0 && variant != 3 ? String.format("%06d", key) : "";
            events.event(PlatformLines.pairing(d.getName(), passkey));
            if (isOrderedBroadcast()) {
                abortBroadcast();
            }
        } else if (BluetoothDevice.ACTION_BOND_STATE_CHANGED.equals(a)) {
            BluetoothDevice d = intent.getParcelableExtra(BluetoothDevice.EXTRA_DEVICE);
            int state = intent.getIntExtra(BluetoothDevice.EXTRA_BOND_STATE, -1);
            int prev = intent.getIntExtra(BluetoothDevice.EXTRA_PREVIOUS_BOND_STATE, -1);
            String name = d == null ? "" : d.getName();
            if (state == BluetoothDevice.BOND_BONDED) {
                events.event(PlatformLines.paired(name, true));
                sendBluetooth();
            } else if (state == BluetoothDevice.BOND_NONE && prev == BluetoothDevice.BOND_BONDING) {
                events.event(PlatformLines.paired(name, false));
            }
        } else if (a != null && a.startsWith("android.net.wifi")) {
            sendWifi();
        } else {
            sendBluetooth();
        }
    }

    @Override
    public void onRequest(String line) {
        PlatformLines.Request r = PlatformLines.parseRequest(line);
        if (r == null) {
            Log.e(TAG, "unknown request " + line);
            return;
        }
        try {
            handle(r);
        } catch (RuntimeException e) {
            Log.e(TAG, "request " + line + " failed", e);
        }
    }

    private void handle(PlatformLines.Request r) {
        boolean on = r.arg.equals("on");
        if (r.kind.equals("wifi") && wifi != null) {
            wifi.setWifiEnabled(on);
        } else if (r.kind.equals("bluetooth") && bt != null) {
            if (on) {
                bt.enable();
            } else {
                bt.disable();
            }
        } else if (r.kind.equals("discoverable") && bt != null) {
            discoverable(Integer.parseInt(r.arg));
        } else if (r.kind.equals("pairing") && pending != null) {
            pending.setPairingConfirmation(r.arg.equals("accept"));
            pending = null;
        } else if (r.kind.equals("provisioned")) {
            Settings.Global.putInt(context.getContentResolver(),
                    Settings.Global.DEVICE_PROVISIONED, 1);
            Settings.Secure.putInt(context.getContentResolver(), "user_setup_complete", 1);
        } else if (r.kind.equals("open")) {
            Intent i = context.getPackageManager().getLaunchIntentForPackage(r.arg);
            if (i == null) {
                Log.e(TAG, "no launcher activity in " + r.arg);
                return;
            }
            i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            context.startActivity(i);
        }
    }

    /**
     * Visible to phones without the system's confirmation dialog (which needs a touch
     * screen): the hidden BluetoothAdapter.setScanMode(int, int), present on 4.4 and 6.0.
     */
    private void discoverable(int seconds) {
        try {
            Method m = BluetoothAdapter.class.getMethod("setScanMode", int.class, int.class);
            m.invoke(bt, SCAN_MODE_DISCOVERABLE, seconds);
        } catch (Exception e) {
            Log.e(TAG, "setScanMode failed", e);
        }
    }
}
