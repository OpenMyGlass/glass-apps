// SPDX-License-Identifier: Apache-2.0
package org.openmyglass.launcher;

import java.util.ArrayList;
import java.util.List;

/**
 * The line format between the native launcher and this wrapper (crates/glass-launcher
 * src/platform.rs, PlatformEvent and PlatformRequest): fields separated by tabs, one
 * message per call. Pure Java, tested on the JVM.
 */
public final class PlatformLines {
    private PlatformLines() {}

    /** A field without the separators of the line format. */
    public static String field(String s) {
        if (s == null) {
            return "";
        }
        return s.replace('\t', ' ').replace('\n', ' ').replace('\r', ' ');
    }

    /** Joins fields into one line. */
    public static String line(String... fields) {
        StringBuilder b = new StringBuilder();
        for (int i = 0; i < fields.length; i++) {
            if (i > 0) {
                b.append('\t');
            }
            b.append(i == 0 ? fields[i] : field(fields[i]));
        }
        return b.toString();
    }

    /** {@code wifi on|off [network]}. */
    public static String wifi(boolean on, String network) {
        if (!on || network == null || network.isEmpty()) {
            return line("wifi", on ? "on" : "off");
        }
        return line("wifi", "on", network);
    }

    /** {@code bluetooth on|off [device...]}. */
    public static String bluetooth(boolean on, List<String> devices) {
        List<String> f = new ArrayList<String>();
        f.add("bluetooth");
        f.add(on ? "on" : "off");
        if (on && devices != null) {
            f.addAll(devices);
        }
        return line(f.toArray(new String[0]));
    }

    public static String btName(String name) {
        return line("bt-name", name);
    }

    /** A pairing request; {@code passkey} is empty for a consent-only pairing. */
    public static String pairing(String device, String passkey) {
        return line("pairing", device == null || device.isEmpty() ? "a phone" : device,
                passkey == null ? "" : passkey);
    }

    public static String paired(String device, boolean ok) {
        return line("paired", device, ok ? "ok" : "failed");
    }

    public static String provisioned(boolean done) {
        return line("provisioned", done ? "true" : "false");
    }

    /** SSIDs come quoted from WifiManager; unknown ones as "<unknown ssid>". */
    public static String ssid(String raw) {
        if (raw == null || raw.equals("<unknown ssid>") || raw.equals("0x")) {
            return "";
        }
        if (raw.length() >= 2 && raw.startsWith("\"") && raw.endsWith("\"")) {
            return raw.substring(1, raw.length() - 1);
        }
        return raw;
    }

    /** A request from the native side. */
    public static final class Request {
        public final String kind;
        public final String arg;

        Request(String kind, String arg) {
            this.kind = kind;
            this.arg = arg;
        }
    }

    /**
     * Parses {@code wifi on|off}, {@code bluetooth on|off}, {@code discoverable SECONDS},
     * {@code pairing accept|reject}, {@code provisioned}, {@code open PACKAGE}; null when
     * malformed.
     */
    public static Request parseRequest(String line) {
        if (line == null) {
            return null;
        }
        String[] f = line.split("\t", -1);
        String k = f[0];
        if (k.equals("provisioned") && f.length == 1) {
            return new Request(k, "");
        }
        if (f.length != 2) {
            return null;
        }
        String a = f[1];
        if ((k.equals("wifi") || k.equals("bluetooth")) && (a.equals("on") || a.equals("off"))) {
            return new Request(k, a);
        }
        if (k.equals("pairing") && (a.equals("accept") || a.equals("reject"))) {
            return new Request(k, a);
        }
        if (k.equals("discoverable")) {
            try {
                int s = Integer.parseInt(a);
                return s >= 0 && s <= 3600 ? new Request(k, a) : null;
            } catch (NumberFormatException e) {
                return null;
            }
        }
        if (k.equals("open") && !a.isEmpty()) {
            return new Request(k, a);
        }
        return null;
    }
}
