// SPDX-License-Identifier: Apache-2.0
package org.openmyglass.launcher;

import java.util.Arrays;

/** JVM tests of PlatformLines, with no test framework: run main, a failure exits 1. */
public final class PlatformLinesTest {
    private static int failures;

    private static void eq(Object expected, Object actual, String what) {
        boolean ok = expected == null ? actual == null : expected.equals(actual);
        if (!ok) {
            failures++;
            System.err.println("FAIL " + what + ": expected <" + expected + "> got <" + actual + ">");
        }
    }

    public static void main(String[] args) {
        eq("wifi\ton\tHome Net", PlatformLines.wifi(true, "Home Net"), "wifi on");
        eq("wifi\toff", PlatformLines.wifi(false, "x"), "wifi off drops the network");
        eq("wifi\ton", PlatformLines.wifi(true, ""), "wifi on without a network");
        eq("bluetooth\ton\tPixel 7\tHead set",
                PlatformLines.bluetooth(true, Arrays.asList("Pixel 7", "Head\tset")), "bluetooth");
        eq("bluetooth\toff", PlatformLines.bluetooth(false, Arrays.asList("x")), "bluetooth off");
        eq("pairing\tPixel\t012345", PlatformLines.pairing("Pixel", "012345"), "pairing");
        eq("pairing\ta phone\t", PlatformLines.pairing(null, null), "consent pairing");
        eq("paired\tPixel\tfailed", PlatformLines.paired("Pixel", false), "paired");
        eq("provisioned\ttrue", PlatformLines.provisioned(true), "provisioned");
        eq("bt-name\tGlass", PlatformLines.btName("Glass"), "bt-name");
        eq("Home", PlatformLines.ssid("\"Home\""), "quoted ssid");
        eq("", PlatformLines.ssid("<unknown ssid>"), "unknown ssid");

        eq("wifi/on", str(PlatformLines.parseRequest("wifi\ton")), "request wifi");
        eq("bluetooth/off", str(PlatformLines.parseRequest("bluetooth\toff")), "request bt");
        eq("discoverable/300", str(PlatformLines.parseRequest("discoverable\t300")), "disc");
        eq("pairing/reject", str(PlatformLines.parseRequest("pairing\treject")), "pairing");
        eq("provisioned/", str(PlatformLines.parseRequest("provisioned")), "provisioned");
        eq("open/com.example", str(PlatformLines.parseRequest("open\tcom.example")), "open");
        for (String bad : new String[] {null, "", "wifi", "wifi\tmaybe", "discoverable\t-1",
                "discoverable\tlots", "discoverable\t99999", "open\t", "provisioned\tx", "x\ty"}) {
            eq(null, PlatformLines.parseRequest(bad), "malformed " + bad);
        }
        if (failures > 0) {
            System.err.println(failures + " failures");
            System.exit(1);
        }
        System.out.println("PlatformLinesTest: ok");
    }

    private static String str(PlatformLines.Request r) {
        return r == null ? null : r.kind + "/" + r.arg;
    }
}
